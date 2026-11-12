# DCL LAN hardware test (Windows, QEMU+WHPX).
#
# Boots the signed Standard guest on this PC and prints a link + password.
# Anyone on the same Wi-Fi/LAN opens the link to reach the XFCE desktop.
# No Matchmaker, gateway, Headscale or renterd: this only proves that the
# hypervisor + guest image + KasmVNC work on real hardware.
#
# Double-click lan-test.cmd. Add -CheckOnly to only check this PC.
# Writes lan-test-report-<time>.txt next to this script (no password in it).

[CmdletBinding()]
param(
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Pinned Weilnetz QEMU (must match host-agent/src-tauri/src/hypervisor.rs and
# windows-host-setup.ps1).
$script:QemuInstallerUrl = "https://qemu.weilnetz.de/w64/qemu-w64-setup-20260811.exe"
$script:QemuInstallerSha512 = "5bcf9eed634e8575a37b74f445af41a2fe4106da512d0c30c368301d4c105037fdfab40a5287367a28a957624cddebbc8c07e16c88ab6634f554cdf3d16bf543"
$script:QemuVersion = "20260811"
# Same cache as windows-host-setup.ps1 so a later real setup reuses the disk.
$script:ImageDir = "C:\dcl\images\standard"
# QEMU argv paths must not contain spaces or commas.
$script:RunDir = "C:\dcl\lan-test"
$script:FirewallRule = "DCL LAN test"
$script:VmName = "dcl-lan-test"

$script:Here = $PSScriptRoot
if (-not $script:Here) { $script:Here = Split-Path -Parent $PSCommandPath }
$script:Stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$script:Report = Join-Path $script:Here "lan-test-report-$($script:Stamp).txt"
$script:Result = "FAIL"
$script:QemuProc = $null

function Write-TestLog {
    param([string]$Message)
    Write-Host $Message
    Add-Content -LiteralPath $script:Report -Value $Message -Encoding utf8
}

function Stop-Test {
    param([string]$Message)
    Write-TestLog "FAIL: $Message"
    throw [System.OperationCanceledException]::new($Message)
}

function Test-IsAdministrator {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Request-Administrator {
    if (Test-IsAdministrator) { return $false }
    Write-Host "Requesting Administrator rights (click Yes on the prompt)..."
    $argList = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"")
    if ($CheckOnly) { $argList += "-CheckOnly" }
    Start-Process -FilePath "powershell.exe" -Verb RunAs -ArgumentList $argList
    Write-Host "The test continues in the new Administrator window."
    return $true
}

function Import-LanTestEnv {
    $path = Join-Path $script:Here "lan-test.env"
    if (-not (Test-Path -LiteralPath $path)) {
        Stop-Test "lan-test.env is missing next to this script."
    }
    $allowed = @(
        "IMAGE_STANDARD_DIGEST", "IMAGE_RAW_URL", "IMAGE_QCOW2_URL",
        "LAN_TEST_PORT", "LAN_TEST_VCPUS", "LAN_TEST_MEMORY_GIB"
    )
    $values = @{}
    foreach ($line in Get-Content -LiteralPath $path) {
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#")) { continue }
        $parts = $trimmed.Split("=", 2)
        if ($parts.Count -ne 2) { continue }
        $key = $parts[0].Trim()
        if ($allowed -notcontains $key) {
            Stop-Test "Unsupported key in lan-test.env: $key"
        }
        $values[$key] = $parts[1].Trim().Trim('"')
    }
    return $values
}

function Get-Setting {
    param([hashtable]$Values, [string]$Key, [string]$Default)
    if ($Values.ContainsKey($Key) -and $Values[$Key]) { return $Values[$Key] }
    return $Default
}

function Get-LanIPv4 {
    $route = Get-NetRoute -DestinationPrefix "0.0.0.0/0" -ErrorAction SilentlyContinue |
        Sort-Object RouteMetric |
        Select-Object -First 1
    if ($route) {
        $addr = Get-NetIPAddress -AddressFamily IPv4 -InterfaceIndex $route.InterfaceIndex -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -notlike "127.*" -and $_.IPAddress -notlike "169.254.*" } |
            Select-Object -First 1
        if ($addr) { return $addr.IPAddress }
    }
    return $null
}

function Stop-StaleTest {
    # A closed window skips cleanup; remove what a previous run left behind.
    Get-CimInstance Win32_Process -Filter "Name = 'qemu-system-x86_64.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -and $_.CommandLine -like "*-name $($script:VmName)*" } |
        ForEach-Object {
            Write-TestLog "Stopping a VM left over from an earlier test (pid $($_.ProcessId))."
            Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
        }
    Get-NetFirewallRule -DisplayName $script:FirewallRule -ErrorAction SilentlyContinue |
        Remove-NetFirewallRule -ErrorAction SilentlyContinue
}

function Assert-Machine {
    param([int]$Vcpus, [int]$MemGib)
    Write-TestLog ""
    Write-TestLog "== Checking this PC =="
    $os = Get-CimInstance Win32_OperatingSystem
    Write-TestLog "OS: $($os.Caption) $($os.Version)"
    if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
        Stop-Test "Only x86-64 Windows is supported (found $($env:PROCESSOR_ARCHITECTURE))."
    }
    $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
    $threads = [int]$cpu.NumberOfLogicalProcessors
    Write-TestLog "CPU: $($cpu.Name.Trim())"
    Write-TestLog "CPU threads: $threads"

    # When a hypervisor already runs (Hyper-V, WSL2, VBS), Windows reports
    # VirtualizationFirmwareEnabled=False even though VT-x/AMD-V is on.
    $cs = Get-CimInstance Win32_ComputerSystem
    if ($cs.HypervisorPresent) {
        Write-TestLog "CPU virtualization: on (a hypervisor is already running)"
    }
    elseif ($cpu.PSObject.Properties["VirtualizationFirmwareEnabled"] -and -not $cpu.VirtualizationFirmwareEnabled) {
        Stop-Test "CPU virtualization is off. Reboot into BIOS/UEFI, enable Intel VT-x or AMD-V/SVM, then run again."
    }
    else {
        Write-TestLog "CPU virtualization: on"
    }

    $whpx = Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform
    if ($whpx.State -ne "Enabled") {
        Write-TestLog "Windows Hypervisor Platform is off. Turning it on..."
        $dism = Start-Process -FilePath "dism.exe" -ArgumentList @(
            "/online", "/Enable-Feature", "/FeatureName:HypervisorPlatform", "/All", "/NoRestart"
        ) -Wait -PassThru -NoNewWindow
        if ($dism.ExitCode -notin 0, 3010) {
            Stop-Test "Could not turn on Windows Hypervisor Platform (DISM exit $($dism.ExitCode))."
        }
        $script:Result = "NEEDS REBOOT (Windows Hypervisor Platform turned on)"
        Write-TestLog ""
        Write-TestLog "Restart the PC, then double-click lan-test.cmd again."
        throw [System.OperationCanceledException]::new("reboot required")
    }
    Write-TestLog "Windows Hypervisor Platform: enabled"

    if ($threads -lt ($Vcpus + 1)) {
        Stop-Test "Need at least $($Vcpus + 1) CPU threads (have $threads)."
    }
    $freeMib = [math]::Floor([double]$os.FreePhysicalMemory / 1024)
    Write-TestLog ("Free RAM: {0:N1} GiB (VM uses {1} GiB)" -f ($freeMib / 1024), $MemGib)
    if ($freeMib -lt ($MemGib * 1024 + 512)) {
        Stop-Test "Not enough free RAM: the test needs about $($MemGib + 1) GiB free. Close other apps and run again."
    }
    $drive = Get-PSDrive -Name C
    $freeGib = [math]::Floor([double]$drive.Free / 1GB)
    Write-TestLog "Free disk on C: $freeGib GiB"
    $imageReady = Test-Path -LiteralPath (Join-Path $script:ImageDir "rootfs.img")
    if (-not $imageReady -and $freeGib -lt 80) {
        Stop-Test "Need about 80 GiB free on C: to unpack the guest disk (have $freeGib GiB)."
    }
}

function Get-QemuInstallDir {
    return Join-Path $env:LOCALAPPDATA "dcl-host-agent\hypervisors\qemu\$($script:QemuVersion)"
}

function Install-PinnedQemu {
    $installDir = Get-QemuInstallDir
    $qemuImg = Join-Path $installDir "qemu-img.exe"
    $qemuSys = Join-Path $installDir "qemu-system-x86_64.exe"
    if ((Test-Path -LiteralPath $qemuImg) -and (Test-Path -LiteralPath $qemuSys)) {
        Write-TestLog "QEMU already installed."
        return $installDir
    }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $setup = Join-Path $installDir "qemu-setup.exe"
    if (-not (Test-Path -LiteralPath $setup)) {
        Write-TestLog "Downloading pinned QEMU installer..."
        curl.exe -L --retry 5 --retry-all-errors -o $setup $script:QemuInstallerUrl
        if ($LASTEXITCODE -ne 0) { Stop-Test "QEMU download failed." }
    }
    $actual = (Get-FileHash -LiteralPath $setup -Algorithm SHA512).Hash.ToLowerInvariant()
    if ($actual -ne $script:QemuInstallerSha512) {
        Remove-Item -LiteralPath $setup -Force
        Stop-Test "QEMU installer checksum mismatch; not installed."
    }
    Write-TestLog "Installing QEMU silently..."
    $setupCmd = $setup -replace '/', '\'
    cmd.exe /c "`"$setupCmd`" /S /D=$installDir"
    if (-not (Test-Path -LiteralPath $qemuSys)) {
        Stop-Test "QEMU installer finished but qemu-system-x86_64.exe is missing."
    }
    return $installDir
}

function Test-RawDigest {
    param([string]$Raw, [string]$Want)
    Write-TestLog "Checking image SHA-256 (takes a minute)..."
    $hash = (Get-FileHash -LiteralPath $Raw -Algorithm SHA256).Hash.ToLowerInvariant()
    return $hash -eq $Want
}

function Install-GuestDisk {
    param([string]$QemuDir, [hashtable]$Settings)
    Write-TestLog ""
    Write-TestLog "== Preparing the guest image =="
    $want = $Settings["IMAGE_STANDARD_DIGEST"].ToLowerInvariant()
    New-Item -ItemType Directory -Force -Path $script:ImageDir | Out-Null
    $raw = Join-Path $script:ImageDir "rootfs.img"
    if (Test-Path -LiteralPath $raw) {
        if (Test-RawDigest -Raw $raw -Want $want) {
            Write-TestLog "Guest image verified."
            return $raw
        }
        Write-TestLog "Cached image does not match; preparing it again."
        Remove-Item -LiteralPath $raw -Force
    }
    $qemuImg = Join-Path $QemuDir "qemu-img.exe"
    $kitRaw = Join-Path $script:Here "rootfs.img"
    $kitQcow = Join-Path $script:Here "rootfs.qcow2"
    $rawUrl = Get-Setting $Settings "IMAGE_RAW_URL" ""
    $qcowUrl = Get-Setting $Settings "IMAGE_QCOW2_URL" ""
    if (Test-Path -LiteralPath $kitRaw) {
        Copy-Item -LiteralPath $kitRaw -Destination $raw -Force
    }
    elseif (Test-Path -LiteralPath $kitQcow) {
        Write-TestLog "Converting rootfs.qcow2 to raw (takes a while)..."
        & $qemuImg convert -p -O raw $kitQcow $raw
        if ($LASTEXITCODE -ne 0) { Stop-Test "qemu-img convert failed." }
    }
    elseif ($rawUrl) {
        Write-TestLog "Downloading guest image (several GB)..."
        curl.exe -L --retry 5 --retry-all-errors -C - -o "$raw.part" $rawUrl
        if ($LASTEXITCODE -ne 0) { Stop-Test "Guest image download failed." }
        Move-Item -LiteralPath "$raw.part" -Destination $raw -Force
    }
    elseif ($qcowUrl) {
        $qcow = Join-Path $script:ImageDir "rootfs.qcow2"
        Write-TestLog "Downloading guest image (several GB)..."
        curl.exe -L --retry 5 --retry-all-errors -C - -o $qcow $qcowUrl
        if ($LASTEXITCODE -ne 0) { Stop-Test "Guest image download failed." }
        Write-TestLog "Converting rootfs.qcow2 to raw (takes a while)..."
        & $qemuImg convert -p -O raw $qcow $raw
        if ($LASTEXITCODE -ne 0) { Stop-Test "qemu-img convert failed." }
        Remove-Item -LiteralPath $qcow -Force -ErrorAction SilentlyContinue
    }
    else {
        Stop-Test "No guest image. Put rootfs.qcow2 next to lan-test.cmd, or set IMAGE_QCOW2_URL in lan-test.env."
    }
    if (-not (Test-RawDigest -Raw $raw -Want $want)) {
        Remove-Item -LiteralPath $raw -Force
        Stop-Test "Guest image SHA-256 does not match lan-test.env. Deleted it; not booting."
    }
    Write-TestLog "Guest image verified."
    return $raw
}

function New-LanPassword {
    $alphabet = "abcdefghjkmnpqrstuvwxyz23456789".ToCharArray()
    $bytes = New-Object byte[] 12
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return -join ($bytes | ForEach-Object { $alphabet[$_ % $alphabet.Length] })
}

function Get-HttpCode {
    param([int]$Port)
    $code = & curl.exe -sk -o NUL -m 3 -w "%{http_code}" "https://127.0.0.1:$Port/" 2>$null
    if ($code -match '^[1-5][0-9][0-9]$') { return $code }
    return $null
}

function Start-LanTest {
    $settings = Import-LanTestEnv
    $digest = Get-Setting $settings "IMAGE_STANDARD_DIGEST" ""
    if ($digest -notmatch '^[0-9a-fA-F]{64}$') {
        Stop-Test "IMAGE_STANDARD_DIGEST in lan-test.env must be 64 hexadecimal characters."
    }
    $port = [int](Get-Setting $settings "LAN_TEST_PORT" "8443")
    $vcpus = [int](Get-Setting $settings "LAN_TEST_VCPUS" "2")
    $memGib = [int](Get-Setting $settings "LAN_TEST_MEMORY_GIB" "4")
    if ($port -lt 1024 -or $port -gt 65535) { Stop-Test "LAN_TEST_PORT must be 1024-65535." }
    if ($vcpus -lt 2) { Stop-Test "LAN_TEST_VCPUS must be >= 2." }
    if ($memGib -lt 4) { Stop-Test "LAN_TEST_MEMORY_GIB must be >= 4." }

    Stop-StaleTest
    Assert-Machine -Vcpus $vcpus -MemGib $memGib

    $lanIp = Get-LanIPv4
    if (-not $lanIp) { Stop-Test "No network connection found. Connect to the Wi-Fi/LAN first." }
    Write-TestLog "LAN address: $lanIp"
    if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) {
        Stop-Test "Port $port is already in use. Set LAN_TEST_PORT=8444 in lan-test.env."
    }

    if ($CheckOnly) {
        $script:Result = "CHECK PASSED (no VM started)"
        Write-TestLog "This PC passes the checks."
        return
    }

    Write-TestLog ""
    Write-TestLog "== Installing QEMU if needed =="
    $qemuDir = Install-PinnedQemu
    $qemuSys = Join-Path $qemuDir "qemu-system-x86_64.exe"
    $raw = Install-GuestDisk -QemuDir $qemuDir -Settings $settings

    Write-TestLog ""
    Write-TestLog "== Starting the virtual machine =="
    New-Item -ItemType Directory -Force -Path $script:RunDir | Out-Null
    $password = New-LanPassword
    $pwFile = Join-Path $script:RunDir "lan-password"
    [System.IO.File]::WriteAllText($pwFile, $password)
    $serialLog = Join-Path $script:RunDir "serial-$($script:Stamp).log"
    $outLog = Join-Path $script:RunDir "qemu-out.log"
    $errLog = Join-Path $script:RunDir "qemu-err.log"

    New-NetFirewallRule -DisplayName $script:FirewallRule -Direction Inbound -Protocol TCP `
        -LocalPort $port -RemoteAddress LocalSubnet -Program $qemuSys -Action Allow -Profile Any | Out-Null
    Write-TestLog "Opened firewall port $port for the local network only."

    # Same flags as host-agent qemu.rs (x86_64 WHPX), plus:
    #  -snapshot  writes go to a temp file; the signed image stays unchanged
    #  hostfwd    publishes guest KasmVNC 8443 on this PC's LAN port
    #  fw_cfg     LAN password -> guest turns on KasmVNC basic auth
    $qemuArgs = @(
        "-name", $script:VmName,
        "-accel", "whpx",
        "-M", "pc",
        "-smp", "cores=$vcpus",
        "-m", "$($memGib)G",
        "-device", "ich9-usb-ehci1",
        "-device", "usb-tablet",
        "-hda", $raw,
        "-snapshot",
        "-display", "none",
        "-monitor", "none",
        "-serial", "file:$serialLog",
        "-netdev", "user,id=net0,hostfwd=tcp:0.0.0.0:$port-:8443",
        "-device", "virtio-net-pci,netdev=net0",
        "-fw_cfg", "name=opt/org.dcl/lan-password,file=$pwFile"
    )
    foreach ($arg in $qemuArgs) {
        if ($arg -match '[\s"]') { Stop-Test "Internal error: QEMU argument has a space: $arg" }
    }
    $bootStart = Get-Date
    $script:QemuProc = Start-Process -FilePath $qemuSys -ArgumentList $qemuArgs -PassThru `
        -NoNewWindow -RedirectStandardOutput $outLog -RedirectStandardError $errLog
    Write-TestLog "VM started (pid $($script:QemuProc.Id)). Waiting for the desktop (up to 5 minutes)..."

    $code = $null
    while (((Get-Date) - $bootStart).TotalSeconds -lt 300) {
        if ($script:QemuProc.HasExited) {
            Write-TestLog "QEMU stopped early. Its output:"
            if (Test-Path -LiteralPath $errLog) {
                Get-Content -LiteralPath $errLog | ForEach-Object { Write-TestLog $_ }
            }
            Stop-Test "The virtual machine did not start. If the output mentions WHPX, restart the PC and try again."
        }
        $code = Get-HttpCode -Port $port
        if ($code) { break }
        Start-Sleep -Seconds 3
    }
    Remove-Item -LiteralPath $pwFile -Force -ErrorAction SilentlyContinue
    $bootSecs = [int]((Get-Date) - $bootStart).TotalSeconds
    if (-not $code) {
        Write-TestLog "Last lines of the guest console ($serialLog):"
        if (Test-Path -LiteralPath $serialLog) {
            Get-Content -LiteralPath $serialLog -Tail 40 | ForEach-Object { Write-TestLog $_ }
        }
        Stop-Test "The desktop did not answer within 5 minutes."
    }
    Write-TestLog "Desktop answered after $($bootSecs)s (HTTP $code)."
    if ($code -ne "401") {
        Write-TestLog "WARNING: expected a password prompt (HTTP 401) but got HTTP $code."
        Write-TestLog "WARNING: the guest image may be older than the LAN password change."
    }
    $script:Result = "PASS: desktop reachable after $($bootSecs)s"

    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Green
    Write-Host "  The test computer is ready." -ForegroundColor Green
    Write-Host ""
    Write-Host "  On another device on the SAME Wi-Fi, open:"
    Write-Host ""
    Write-Host "      https://$($lanIp):$port" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "  The browser warns about the certificate: choose"
    Write-Host "  `"Advanced`" -> `"Continue`" (it is a self-signed test cert)."
    Write-Host ""
    Write-Host "  Username:  ubuntu"
    Write-Host "  Password:  $password" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "  Keep this window open. Press Ctrl+C to stop the test."
    Write-Host "============================================================" -ForegroundColor Green
    Write-Host ""
    Write-TestLog "Link shown: https://$($lanIp):$port (password not saved)"

    while (-not $script:QemuProc.HasExited) {
        Start-Sleep -Seconds 2
    }
    $script:QemuProc = $null
    Write-TestLog "The virtual machine stopped by itself."
}

# --- main -------------------------------------------------------------------
if (Request-Administrator) { exit 0 }

Set-Content -LiteralPath $script:Report -Value "DCL LAN hardware test (Windows)" -Encoding utf8
Write-TestLog "Started: $(Get-Date -Format o)"
$exitCode = 0
try {
    Start-LanTest
}
catch [System.OperationCanceledException] {
    $exitCode = 1
}
catch {
    Write-TestLog "FAIL: $($_.Exception.Message)"
    $exitCode = 1
}
finally {
    if ($script:Result -eq "FAIL" -and $exitCode -eq 0) {
        $script:Result = "STOPPED by user before the desktop was ready"
    }
    if ($script:QemuProc -and -not $script:QemuProc.HasExited) {
        Write-TestLog "Stopping the virtual machine..."
        Stop-Process -Id $script:QemuProc.Id -Force -ErrorAction SilentlyContinue
    }
    Get-NetFirewallRule -DisplayName $script:FirewallRule -ErrorAction SilentlyContinue |
        Remove-NetFirewallRule -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath (Join-Path $script:RunDir "lan-password") -Force -ErrorAction SilentlyContinue
    Write-TestLog "Result: $($script:Result)"
    Write-TestLog "Finished: $(Get-Date -Format o)"
    Write-Host ""
    Write-Host "Report saved to: $($script:Report)"
    Write-Host "Send that file back to the project owner."
    Read-Host "Press Enter to close" | Out-Null
}
exit $exitCode
