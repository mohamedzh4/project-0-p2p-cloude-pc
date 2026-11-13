# DCL LAN hardware test (Windows, QEMU+WHPX).
#
# Boots the signed Standard guest on this PC and prints a link + password.
# Anyone on the same Wi-Fi/LAN opens the link to reach the XFCE desktop.
# No Matchmaker, gateway, Headscale or renterd: this only proves that the
# hypervisor + guest image + KasmVNC work on real hardware.
#
# Double-click lan-test.cmd. Add -CheckOnly to only check this PC.
#
# Two modes (lan-test.env LAN_TEST_MODE, default picked automatically):
#   quick   no image build needed: official Ubuntu 24.04 cloud image
#           (pinned SHA-256) + seed.iso from the kit; the first boot installs
#           XFCE + KasmVNC with the same provision-kasm.sh as the real image.
#   signed  the Packer-built Standard guest (IMAGE_STANDARD_DIGEST set).
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
# Quick mode base image (must match lan-test.sh). Dated folder: the bare
# release/ link moves.
$script:UbuntuUrl = "https://cloud-images.ubuntu.com/releases/noble/release-20261017/ubuntu-24.04-server-cloudimg-amd64.img"
$script:UbuntuSha256 = "04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94"
$script:QuickDir = "C:\dcl\lan-test\quick"
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
        "LAN_TEST_PORT", "LAN_TEST_VCPUS", "LAN_TEST_MEMORY_GIB",
        "LAN_TEST_MODE", "LAN_TEST_SETUP_MINUTES", "LAN_TEST_BOOT_MINUTES"
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
    param([int]$Vcpus, [int]$MemGib, [string]$Mode)
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
    if ($Mode -eq "quick") {
        if ($freeGib -lt 20) { Stop-Test "Need about 20 GiB free on C: (have $freeGib GiB)." }
    }
    elseif (-not $imageReady -and $freeGib -lt 80) {
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

function Install-QuickDisk {
    param([string]$QemuDir)
    Write-TestLog ""
    Write-TestLog "== Preparing the guest image (quick mode) =="
    New-Item -ItemType Directory -Force -Path $script:QuickDir | Out-Null
    $base = Join-Path $script:QuickDir ("ubuntu-24.04-cloudimg-" + $script:UbuntuSha256.Substring(0, 12) + ".img")
    $overlay = Join-Path $script:QuickDir "overlay.qcow2"
    $seed = Join-Path $script:QuickDir "seed.iso"
    $doneMark = Join-Path $script:QuickDir "setup-done"
    if (-not (Test-Path -LiteralPath $base)) {
        Write-TestLog "Downloading Ubuntu 24.04 cloud image (about 600 MB)..."
        curl.exe -L --retry 5 --retry-all-errors -C - -o "$base.part" $script:UbuntuUrl
        if ($LASTEXITCODE -ne 0) { Stop-Test "Ubuntu image download failed. Check the internet connection." }
        $hash = (Get-FileHash -LiteralPath "$base.part" -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -ne $script:UbuntuSha256) {
            Remove-Item -LiteralPath "$base.part" -Force
            Stop-Test "Ubuntu image SHA-256 does not match the pinned value. Deleted it; not booting."
        }
        Move-Item -LiteralPath "$base.part" -Destination $base -Force
    }
    Write-TestLog "Ubuntu image verified ($($script:UbuntuSha256))."
    $kitSeed = Join-Path $script:Here "seed.iso"
    $seedSum = (Get-FileHash -LiteralPath $kitSeed -Algorithm SHA256).Hash.ToLowerInvariant()
    if (Test-Path -LiteralPath $overlay) {
        $marked = $null
        if (Test-Path -LiteralPath $doneMark) { $marked = (Get-Content -LiteralPath $doneMark -Raw).Trim() }
        if ($marked -ne $seedSum) {
            Write-TestLog "Previous setup was unfinished or the kit changed; starting the desktop setup again."
            Remove-Item -LiteralPath $overlay, $doneMark -Force -ErrorAction SilentlyContinue
        }
    }
    Copy-Item -LiteralPath $kitSeed -Destination $seed -Force
    $first = $false
    if (-not (Test-Path -LiteralPath $overlay)) {
        $qemuImg = Join-Path $QemuDir "qemu-img.exe"
        & $qemuImg create -q -f qcow2 -F qcow2 -b $base $overlay 20G
        if ($LASTEXITCODE -ne 0) { Stop-Test "qemu-img could not create the VM disk." }
        $first = $true
    }
    return @{ Overlay = $overlay; Seed = $seed; DoneMark = $doneMark; SeedSum = $seedSum; First = $first }
}

function Start-LanTest {
    $settings = Import-LanTestEnv
    $digest = Get-Setting $settings "IMAGE_STANDARD_DIGEST" ""
    $mode = Get-Setting $settings "LAN_TEST_MODE" ""
    if (-not $mode) {
        if ($digest) { $mode = "signed" } else { $mode = "quick" }
    }
    if ($mode -eq "signed") {
        if ($digest -notmatch '^[0-9a-fA-F]{64}$') {
            Stop-Test "IMAGE_STANDARD_DIGEST in lan-test.env must be 64 hexadecimal characters (signed mode)."
        }
    }
    elseif ($mode -eq "quick") {
        if (-not (Test-Path -LiteralPath (Join-Path $script:Here "seed.iso"))) {
            Stop-Test "seed.iso is missing next to lan-test.cmd (quick mode)."
        }
    }
    else {
        Stop-Test "LAN_TEST_MODE must be quick or signed."
    }
    $setupMin = [int](Get-Setting $settings "LAN_TEST_SETUP_MINUTES" "45")
    if ($setupMin -lt 5) { Stop-Test "LAN_TEST_SETUP_MINUTES must be >= 5." }
    $bootMin = [int](Get-Setting $settings "LAN_TEST_BOOT_MINUTES" "10")
    if ($bootMin -lt 2) { Stop-Test "LAN_TEST_BOOT_MINUTES must be >= 2." }
    $port = [int](Get-Setting $settings "LAN_TEST_PORT" "8443")
    $vcpus = [int](Get-Setting $settings "LAN_TEST_VCPUS" "2")
    $memGib = [int](Get-Setting $settings "LAN_TEST_MEMORY_GIB" "4")
    if ($port -lt 1024 -or $port -gt 65535) { Stop-Test "LAN_TEST_PORT must be 1024-65535." }
    if ($vcpus -lt 2) { Stop-Test "LAN_TEST_VCPUS must be >= 2." }
    if ($memGib -lt 4) { Stop-Test "LAN_TEST_MEMORY_GIB must be >= 4." }

    Stop-StaleTest
    Write-TestLog "Mode: $mode"
    Assert-Machine -Vcpus $vcpus -MemGib $memGib -Mode $mode

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
    $quick = $null
    if ($mode -eq "signed") {
        $raw = Install-GuestDisk -QemuDir $qemuDir -Settings $settings
        $diskArgs = @("-hda", $raw, "-snapshot")
    }
    else {
        $quick = Install-QuickDisk -QemuDir $qemuDir
        $diskArgs = @("-hda", $quick.Overlay, "-drive", "file=$($quick.Seed),media=cdrom,readonly=on")
    }

    Write-TestLog ""
    Write-TestLog "== Starting the virtual machine =="
    New-Item -ItemType Directory -Force -Path $script:RunDir | Out-Null
    $password = New-LanPassword
    $pwFile = Join-Path $script:RunDir "lan-password"
    # SMBIOS OEM string, not fw_cfg: Ubuntu's cloud kernel has no qemu_fw_cfg.
    [System.IO.File]::WriteAllText($pwFile, "dcl-lan-password=$password")
    $serialLog = Join-Path $script:RunDir "serial-$($script:Stamp).log"
    $outLog = Join-Path $script:RunDir "qemu-out.log"
    $errLog = Join-Path $script:RunDir "qemu-err.log"

    New-NetFirewallRule -DisplayName $script:FirewallRule -Direction Inbound -Protocol TCP `
        -LocalPort $port -RemoteAddress LocalSubnet -Program $qemuSys -Action Allow -Profile Any | Out-Null
    Write-TestLog "Opened firewall port $port for the local network only."

    # Same flags as host-agent qemu.rs (x86_64 WHPX), plus:
    #  -snapshot  (signed) writes go to a temp file; the image is unchanged
    #  overlay    (quick) desktop install persists so later runs boot fast
    #  hostfwd    publishes guest KasmVNC 8443 on this PC's LAN port
    #  smbios 11  LAN password (from a file, not argv) -> KasmVNC basic auth on
    $qemuArgs = @(
        "-name", $script:VmName,
        "-accel", "whpx",
        "-M", "pc",
        "-smp", "cores=$vcpus",
        "-m", "$($memGib)G",
        "-device", "ich9-usb-ehci1",
        "-device", "usb-tablet",
        $diskArgs,
        "-display", "none",
        "-monitor", "none",
        "-serial", "file:$serialLog",
        "-netdev", "user,id=net0,hostfwd=tcp:0.0.0.0:$port-:8443",
        "-device", "virtio-net-pci,netdev=net0",
        "-smbios", "type=11,path=$pwFile"
    )
    $qemuArgs = @($qemuArgs | ForEach-Object { $_ })
    foreach ($arg in $qemuArgs) {
        if ($arg -match '[\s"]') { Stop-Test "Internal error: QEMU argument has a space: $arg" }
    }
    $bootStart = Get-Date
    $script:QemuProc = Start-Process -FilePath $qemuSys -ArgumentList $qemuArgs -PassThru `
        -NoNewWindow -RedirectStandardOutput $outLog -RedirectStandardError $errLog
    $limit = $bootMin * 60
    if ($quick -and $quick.First) {
        $limit = $setupMin * 60
        Write-TestLog "VM started (pid $($script:QemuProc.Id)). First run: installing the desktop inside the VM."
        Write-TestLog "This downloads ~500 MB and takes 10-30 minutes (limit $setupMin min)."
    }
    else {
        Write-TestLog "VM started (pid $($script:QemuProc.Id)). Waiting for the desktop (up to $bootMin minutes)..."
    }

    $code = $null
    $nextNote = 60
    while (((Get-Date) - $bootStart).TotalSeconds -lt $limit) {
        $elapsed = ((Get-Date) - $bootStart).TotalSeconds
        if ($elapsed -ge $nextNote) {
            Write-Host ("  ... still starting ({0} min)" -f [int][math]::Floor($elapsed / 60))
            $nextNote += 60
        }
        $serialText = Get-Content -LiteralPath $serialLog -Raw -ErrorAction SilentlyContinue
        if ($serialText -and $serialText.Contains("DCL-QUICK-SETUP-FAILED")) {
            Write-TestLog "Desktop setup inside the VM failed. Guest console:"
            Get-Content -LiteralPath $serialLog -Tail 60 -ErrorAction SilentlyContinue | ForEach-Object { Write-TestLog $_ }
            Stop-Process -Id $script:QemuProc.Id -Force -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 2
            if ($quick) { Remove-Item -LiteralPath $quick.Overlay -Force -ErrorAction SilentlyContinue }
            Stop-Test "Desktop setup failed inside the VM (see above)."
        }
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
            Get-Content -LiteralPath $serialLog -Tail 80 -ErrorAction SilentlyContinue | ForEach-Object { Write-TestLog $_ }
        }
        if ($quick -and -not $quick.First) {
            # Known open issue: the installed desktop may not come back on a
            # later boot. Drop the marker so the next run reinstalls.
            Remove-Item -LiteralPath $quick.DoneMark -Force -ErrorAction SilentlyContinue
            Write-TestLog "Run lan-test again: it will reinstall the desktop (first-run time)."
        }
        Stop-Test ("The desktop did not answer within {0} minutes." -f [int]($limit / 60))
    }
    Write-TestLog "Desktop answered after $($bootSecs)s (HTTP $code)."
    if ($code -ne "401") {
        # Fail closed: the port is open to the LAN and the desktop has no password.
        Write-TestLog "The desktop answered HTTP $code instead of asking for a password (401)."
        Write-TestLog "The guest image is older than the LAN password change, or the password did not reach the VM."
        Stop-Test "Stopped the VM so the desktop is not open to the network without a password."
    }
    if ($quick -and -not (Test-Path -LiteralPath $quick.DoneMark)) {
        Set-Content -LiteralPath $quick.DoneMark -Value $quick.SeedSum -Encoding ascii -NoNewline
    }
    $script:Result = "PASS ($mode mode): desktop reachable after $($bootSecs)s"

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
