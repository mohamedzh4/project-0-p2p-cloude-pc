# Volunteer Standard host bootstrap (QEMU+WHPX).
# Double-click setup.cmd (or run this file). It self-elevates.
# This cannot flip BIOS virtualization, click UAC, or punch through CGNAT.

[CmdletBinding()]
param(
    [string]$BootstrapToken = "",
    [string]$MatchmakerUrl = "http://lab.example.com:8000",
    [string]$HeadscaleUrl = "http://lab.example.com:8080",
    [string]$ImageSigningPublicKey = "684464c008f401dae13e7ada2f14c6fc8f578ae2d5a7b562868cf9b470d9d764",
    [string]$ImageStandardDigest = "dc41e8e11014a250701aff5b28b19137de0625cd0ac84b91bfb995ba91601b3b",
    [string]$ImageStandardSignature = "h+R4TqINBJRu0FaxSD8qmwe8QunC8U3rp5d7bEecJGiumZvgr7lmtF5WIRvKuXnv/Nf7J/PL9CE7hqW8X+NTBw==",
    [string]$HostDisplayName = "",
    [string]$ImageQcow2Url = "",
    [string]$HostAgentInstallerUrl = "",
    [switch]$ResumeAfterReboot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Pinned Weilnetz QEMU (must match host-agent/src-tauri/src/hypervisor.rs).
$script:QemuInstallerUrl = "https://qemu.weilnetz.de/w64/qemu-w64-setup-20260811.exe"
$script:QemuInstallerSha512 = "5bcf9eed634e8575a37b74f445af41a2fe4106da512d0c30c368301d4c105037fdfab40a5287367a28a957624cddebbc8c07e16c88ab6634f554cdf3d16bf543"
$script:QemuVersion = "20260811"
$script:StableRoot = "C:\dcl\setup"
$script:ImageDir = "C:\dcl\images\standard"
$script:ManualSteps = New-Object System.Collections.Generic.List[string]

function Import-SidecarEnv {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }
    foreach ($line in Get-Content -LiteralPath $Path) {
        $trim = $line.Trim()
        if ($trim -eq "" -or $trim.StartsWith("#") -or $trim -notmatch "=") {
            continue
        }
        $key, $value = $trim.Split("=", 2)
        $key = $key.Trim()
        $value = $value.Trim().Trim('"')
        switch ($key) {
            "HOST_BOOTSTRAP_TOKEN" { if (-not $script:BootstrapToken) { $script:BootstrapToken = $value } }
            "MATCHMAKER_URL" { $script:MatchmakerUrl = $value }
            "HEADSCALE_LOGIN_SERVER" { $script:HeadscaleUrl = $value }
            "IMAGE_SIGNING_PUBLIC_KEY" { $script:ImageSigningPublicKey = $value }
            "IMAGE_STANDARD_DIGEST" { $script:ImageStandardDigest = $value }
            "IMAGE_LOCAL_DIGEST" { $script:ImageStandardDigest = $value }
            "IMAGE_STANDARD_SIGNATURE" { $script:ImageStandardSignature = $value }
            "HOST_DISPLAY_NAME" { if (-not $script:HostDisplayName) { $script:HostDisplayName = $value } }
            "IMAGE_QCOW2_URL" { if (-not $script:ImageQcow2Url) { $script:ImageQcow2Url = $value } }
            "HOST_AGENT_INSTALLER_URL" { if (-not $script:HostAgentInstallerUrl) { $script:HostAgentInstallerUrl = $value } }
            "HOST_AGENT_URL" { if (-not $script:AgentUrlOverride) { $script:AgentUrlOverride = $value } }
            "HOST_AGENT_LISTEN_ADDR" { if (-not $script:ListenAddr) { $script:ListenAddr = $value } }
            "TUNNEL_SSH" { if (-not $script:TunnelSsh) { $script:TunnelSsh = $value } }
            "TUNNEL_REMOTE_PORT" { if (-not $script:TunnelRemotePort) { $script:TunnelRemotePort = $value } }
        }
    }
}

function Test-IsAdministrator {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($id)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Copy-KitToStableRoot {
    New-Item -ItemType Directory -Force -Path $script:StableRoot | Out-Null
    $here = $PSScriptRoot
    if (-not $here) {
        $here = Split-Path -Parent $PSCommandPath
    }
    $names = @(
        "windows-host-setup.ps1",
        "setup.cmd",
        "friend.env",
        "host-agent-setup.exe",
        "start-tunnel.cmd",
        "tunnel.pem"
    )
    foreach ($name in $names) {
        $src = Join-Path $here $name
        if (Test-Path -LiteralPath $src) {
            Copy-Item -LiteralPath $src -Destination (Join-Path $script:StableRoot $name) -Force
        }
    }
    Get-ChildItem -LiteralPath $here -Filter "*nsis*.exe" -ErrorAction SilentlyContinue | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $script:StableRoot "host-agent-setup.exe") -Force
    }
    Set-Content -LiteralPath (Join-Path $script:StableRoot "kit-origin.txt") -Value $here -Encoding ascii
}

function Request-Administrator {
    if (Test-IsAdministrator) {
        return
    }
    Write-Host "Requesting Administrator (click Yes on UAC)..."
    $script:ManualSteps.Add("Click Yes on the User Account Control prompt.")
    $ps1 = Join-Path $script:StableRoot "windows-host-setup.ps1"
    if (-not (Test-Path -LiteralPath $ps1)) {
        $ps1 = $PSCommandPath
    }
    $argList = @(
        "-NoProfile",
        "-ExecutionPolicy", "Bypass",
        "-File", $ps1
    )
    if ($ResumeAfterReboot) { $argList += "-ResumeAfterReboot" }
    Start-Process -FilePath "powershell.exe" -Verb RunAs -ArgumentList $argList
    exit 0
}

function Get-PublicIPv4 {
    $urls = @(
        "https://api.ipify.org",
        "https://ifconfig.me/ip",
        "https://ipv4.icanhazip.com"
    )
    foreach ($url in $urls) {
        try {
            $ip = (Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 8).Content.Trim()
            if ($ip -match '^\d{1,3}(\.\d{1,3}){3}$') {
                return $ip
            }
        }
        catch {
            continue
        }
    }
    throw "Could not detect a public IPv4. Set HOST_AGENT_URL by hand after you know the IP."
}

function Test-CgnatAddress {
    param([string]$Ip)
    $parts = $Ip.Split(".") | ForEach-Object { [int]$_ }
    return ($parts[0] -eq 100 -and $parts[1] -ge 64 -and $parts[1] -le 127)
}

function Get-LanIPv4 {
    $route = Get-NetRoute -DestinationPrefix "0.0.0.0/0" -ErrorAction SilentlyContinue |
        Sort-Object RouteMetric |
        Select-Object -First 1
    if ($route) {
        $addr = Get-NetIPAddress -AddressFamily IPv4 -InterfaceIndex $route.InterfaceIndex -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -notlike "127.*" } |
            Select-Object -First 1
        if ($addr) {
            return $addr.IPAddress
        }
    }
    return $null
}

function Assert-FirmwareVirtualization {
    # When a hypervisor already runs (Hyper-V, WSL2, Memory Integrity/VBS),
    # Windows reports VirtualizationFirmwareEnabled=False even though
    # VT-x/AMD-V is on. Only trust the flag when no hypervisor is present.
    $system = Get-CimInstance -ClassName Win32_ComputerSystem
    if ($system.HypervisorPresent) {
        return
    }
    $cpu = Get-CimInstance -ClassName Win32_Processor | Select-Object -First 1
    $prop = $cpu.PSObject.Properties["VirtualizationFirmwareEnabled"]
    if ($prop -and ($cpu.VirtualizationFirmwareEnabled -eq $false)) {
        throw "Virtualization is off in firmware. Reboot, enable Intel VT-x or AMD-V / SVM in BIOS/UEFI, then run setup.cmd again. Software cannot flip that switch."
    }
}

function Assert-DiskSpace {
    $drive = Get-PSDrive -Name C
    $free = [int64]($drive.Free)
    # 16 GiB raw disk + the agent's verified copy + a session's temporary
    # changes (QEMU snapshot) + the 1-2 GiB compressed download.
    $need = [int64]45GB
    if ($free -lt $need) {
        throw ("C: has {0:N1} GiB free; need about 45 GiB (guest disk, its verified copy, session changes). Free space and re-run." -f ($free / 1GB))
    }
    if ($free -lt 60GB) {
        Write-Host ("Warning: {0:N1} GiB free. A long session that writes a lot may run short." -f ($free / 1GB))
    }
}

function Enable-Whpx {
    Write-Host "Enabling Windows Hypervisor Platform..."
    $dism = Start-Process -FilePath "dism.exe" -ArgumentList @(
        "/online", "/Enable-Feature", "/FeatureName:HypervisorPlatform", "/All", "/NoRestart"
    ) -Wait -PassThru
    if ($dism.ExitCode -notin 0, 3010) {
        throw "DISM HypervisorPlatform failed with exit $($dism.ExitCode)"
    }
    return $dism.ExitCode
}

function Register-ResumeAfterReboot {
    $ps1 = Join-Path $script:StableRoot "windows-host-setup.ps1"
    $cmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$ps1`" -ResumeAfterReboot"
    New-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce" `
        -Name "DclHostSetup" -Value $cmd -PropertyType String -Force | Out-Null
    Write-Host "Reboot required for WHPX. Setup will continue automatically after you log in."
    $script:ManualSteps.Add("Reboot when Windows asks, then log in (setup continues by itself).")
}

function Enable-HostFirewall {
    Write-Host "Opening inbound TCP 7842..."
    if (-not (Get-NetFirewallRule -DisplayName "DCL Host Agent" -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule -DisplayName "DCL Host Agent" -Direction Inbound -Protocol TCP -LocalPort 7842 -Action Allow | Out-Null
    }
}

function Install-TailscaleClient {
    if (Get-Command tailscale -ErrorAction SilentlyContinue) {
        return
    }
    Write-Host "Installing Tailscale (do not log in to tailscale.com)..."
    winget install --id Tailscale.Tailscale -e --accept-package-agreements --accept-source-agreements
}

function Install-WebView2 {
    $marker = Join-Path ${env:ProgramFiles(x86)} "Microsoft\EdgeWebView\Application"
    if (Test-Path -LiteralPath $marker) {
        return
    }
    Write-Host "Installing Edge WebView2 Runtime..."
    winget install --id Microsoft.EdgeWebView2Runtime -e --accept-package-agreements --accept-source-agreements
}

function Get-QemuInstallDir {
    return Join-Path $env:LOCALAPPDATA "dcl-host-agent\hypervisors\qemu\$($script:QemuVersion)"
}

function Install-PinnedQemu {
    $installDir = Get-QemuInstallDir
    $qemuImg = Join-Path $installDir "qemu-img.exe"
    $qemuSys = Join-Path $installDir "qemu-system-x86_64.exe"
    if ((Test-Path -LiteralPath $qemuImg) -and (Test-Path -LiteralPath $qemuSys)) {
        Write-Host "QEMU already present."
        return $installDir
    }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $setup = Join-Path $installDir "qemu-setup.exe"
    if (-not (Test-Path -LiteralPath $setup)) {
        Write-Host "Downloading pinned QEMU installer..."
        curl.exe -L --retry 5 --retry-all-errors -o $setup $script:QemuInstallerUrl
        if ($LASTEXITCODE -ne 0) {
            throw "QEMU download failed."
        }
    }
    $actual = (Get-FileHash -LiteralPath $setup -Algorithm SHA512).Hash.ToLowerInvariant()
    if ($actual -ne $script:QemuInstallerSha512) {
        Remove-Item -LiteralPath $setup -Force
        throw "QEMU installer checksum mismatch; not installed."
    }
    Write-Host "Installing QEMU silently. Click Yes if UAC appears..."
    $script:ManualSteps.Add("Click Yes if Windows asks to allow the QEMU installer.")
    $setupCmd = $setup -replace '/', '\'
    cmd.exe /c "`"$setupCmd`" /S /D=$installDir"
    if (-not (Test-Path -LiteralPath $qemuImg)) {
        throw "QEMU installer finished but qemu-img.exe is missing. Approve UAC and re-run setup.cmd."
    }
    return $installDir
}

function Get-QcowSource {
    $origin = $PSScriptRoot
    $originFile = Join-Path $script:StableRoot "kit-origin.txt"
    if (Test-Path -LiteralPath $originFile) {
        $origin = (Get-Content -LiteralPath $originFile -Raw).Trim()
    }
    $localNames = @(
        (Join-Path $origin "rootfs.qcow2"),
        (Join-Path $script:StableRoot "rootfs.qcow2"),
        (Join-Path $PSScriptRoot "rootfs.qcow2"),
        (Join-Path $script:ImageDir "rootfs.qcow2")
    )
    foreach ($path in $localNames) {
        if (Test-Path -LiteralPath $path) {
            return @{ Kind = "file"; Path = $path }
        }
    }
    if ($script:ImageQcow2Url) {
        return @{ Kind = "url"; Path = $script:ImageQcow2Url }
    }
    return $null
}

function Install-GuestDisk {
    param([string]$QemuDir)
    New-Item -ItemType Directory -Force -Path $script:ImageDir | Out-Null
    $raw = Join-Path $script:ImageDir "rootfs.img"
    if (Test-Path -LiteralPath $raw) {
        $hash = (Get-FileHash -LiteralPath $raw -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($hash -eq $script:ImageStandardDigest.ToLowerInvariant()) {
            Write-Host "Guest disk already verified."
            return
        }
        Write-Host "Existing rootfs.img digest does not match; converting again."
        Remove-Item -LiteralPath $raw -Force
    }
    $src = Get-QcowSource
    if (-not $src) {
        throw "No guest disk. Put rootfs.qcow2 next to setup.cmd, or set IMAGE_QCOW2_URL in friend.env."
    }
    $qcow = Join-Path $script:ImageDir "rootfs.qcow2"
    if ($src.Kind -eq "url") {
        Write-Host "Downloading guest disk (several GB, leave the PC on)..."
        curl.exe -L --retry 5 --retry-all-errors -C - -o $qcow $src.Path
        if ($LASTEXITCODE -ne 0) {
            throw "Guest disk download failed."
        }
    }
    elseif ($src.Path -ne $qcow) {
        Write-Host "Copying guest disk into $script:ImageDir ..."
        Copy-Item -LiteralPath $src.Path -Destination $qcow -Force
    }
    $qemuImg = Join-Path $QemuDir "qemu-img.exe"
    Write-Host "Converting qcow2 to signed raw rootfs.img (this takes a while)..."
    & $qemuImg convert -p -O raw $qcow $raw
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $raw)) {
        throw "qemu-img convert failed."
    }
    $hash = (Get-FileHash -LiteralPath $raw -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($hash -ne $script:ImageStandardDigest.ToLowerInvariant()) {
        Remove-Item -LiteralPath $raw -Force
        throw "Converted rootfs.img SHA-256 does not match IMAGE_STANDARD_DIGEST. Not using this file."
    }
    Remove-Item -LiteralPath $qcow -Force -ErrorAction SilentlyContinue
    Write-Host "Guest disk verified."
}

function Find-HostAgentInstaller {
    $candidates = @(
        (Join-Path $script:StableRoot "host-agent-setup.exe"),
        (Join-Path $PSScriptRoot "host-agent-setup.exe")
    )
    foreach ($path in $candidates) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }
    $nsis = Get-ChildItem -LiteralPath $PSScriptRoot -Filter "*nsis*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($nsis) {
        return $nsis.FullName
    }
    if ($script:HostAgentInstallerUrl) {
        $dest = Join-Path $script:StableRoot "host-agent-setup.exe"
        Write-Host "Downloading Host Agent installer..."
        curl.exe -L --retry 5 --retry-all-errors -o $dest $script:HostAgentInstallerUrl
        if ($LASTEXITCODE -ne 0) {
            throw "Host Agent installer download failed."
        }
        return $dest
    }
    return $null
}

function Install-HostAgentApp {
    $existing = @(
        (Join-Path $env:LOCALAPPDATA "distributedcomputelab.hostagent\Host Agent.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\Host Agent\Host Agent.exe"),
        (Join-Path $env:ProgramFiles "Host Agent\Host Agent.exe")
    )
    foreach ($path in $existing) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }
    $setup = Find-HostAgentInstaller
    if (-not $setup) {
        throw "Host Agent installer is missing. The person who sent you this folder must include host-agent-setup.exe (built on Windows, or a GitHub Actions NSIS artifact)."
    }
    Write-Host "Installing Host Agent. Click Yes if UAC appears..."
    $script:ManualSteps.Add("Click Yes if Windows asks to allow Host Agent setup.")
    $setupCmd = $setup -replace '/', '\'
    cmd.exe /c "`"$setupCmd`" /S"
    Start-Sleep -Seconds 3
    foreach ($path in $existing) {
        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }
    $found = Get-ChildItem -Path $env:LOCALAPPDATA -Filter "Host Agent.exe" -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($found) {
        return $found.FullName
    }
    throw "Host Agent installer ran but Host Agent.exe was not found. Re-run setup.cmd after approving UAC."
}

function Write-HostAgentEnv {
    param(
        [string]$AgentUrl,
        [string]$ListenAddr
    )
    $appDir = Join-Path $env:APPDATA "com.distributedcomputelab.hostagent"
    New-Item -ItemType Directory -Force -Path $appDir | Out-Null
    $envPath = Join-Path $appDir ".env"
    $displayLine = ""
    if (-not [string]::IsNullOrWhiteSpace($script:HostDisplayName)) {
        $displayLine = "HOST_DISPLAY_NAME=$($script:HostDisplayName)"
    }
    # Left out when empty: the agent then fetches its commands itself.
    $listenLine = ""
    $agentUrlLine = ""
    if (-not [string]::IsNullOrWhiteSpace($AgentUrl)) {
        $listenLine = "HOST_AGENT_LISTEN_ADDR=$ListenAddr"
        $agentUrlLine = "HOST_AGENT_URL=$AgentUrl"
    }
    $envText = @"
MATCHMAKER_URL=$($script:MatchmakerUrl)
HOST_BOOTSTRAP_TOKEN=$($script:BootstrapToken)
$displayLine
$listenLine
$agentUrlLine
HEADSCALE_LOGIN_SERVER=$($script:HeadscaleUrl)
IMAGE_SIGNING_PUBLIC_KEY=$($script:ImageSigningPublicKey)
IMAGE_LOCAL_DIGEST=$($script:ImageStandardDigest)
IMAGE_STANDARD_DIGEST=$($script:ImageStandardDigest)
IMAGE_STANDARD_SIGNATURE=$($script:ImageStandardSignature)
HOST_AGENT_STANDARD_IMAGE_URL=file:///C:/dcl/images/standard/rootfs.img
"@
    Set-Content -Path $envPath -Value $envText.Trim() -Encoding ascii
    Write-Host "Wrote $envPath"
}

function Try-UpnpPort7842 {
    param([string]$LanIp)
    if (-not $LanIp) {
        return $false
    }
    try {
        $nat = New-Object -ComObject HNetCfg.NATUPnP
        $maps = $nat.StaticPortMappingCollection
        if (-not $maps) {
            return $false
        }
        $maps.Add(7842, "TCP", 7842, $LanIp, $true, "DCL Host Agent") | Out-Null
        Write-Host "Asked the router (UPnP) to forward TCP 7842."
        return $true
    }
    catch {
        return $false
    }
}

function Disable-AcSleep {
    powercfg /change standby-timeout-ac 0 | Out-Null
    powercfg /change hibernate-timeout-ac 0 | Out-Null
    Write-Host "Disabled sleep/hibernate while plugged in (so a session is not cut by idle)."
}

function Start-ReverseTunnel {
    if ([string]::IsNullOrWhiteSpace($script:TunnelSsh)) {
        return
    }
    $key = Join-Path $script:StableRoot "tunnel.pem"
    if (-not (Test-Path -LiteralPath $key)) {
        $key = Join-Path $PSScriptRoot "tunnel.pem"
    }
    if (-not (Test-Path -LiteralPath $key)) {
        $script:ManualSteps.Add("Tunnel key tunnel.pem is missing. Azure cannot dial this PC until the reverse SSH tunnel is up.")
        return
    }
    icacls $key /inheritance:r /grant:r "$($env:USERNAME):(R)" | Out-Null
    $remotePort = $script:TunnelRemotePort
    if (-not $remotePort) { $remotePort = "17842" }
    $listen = $script:ListenAddr
    if (-not $listen) { $listen = "127.0.0.1:7842" }
    $local = $listen
    Write-Host "Starting reverse SSH tunnel so Azure Matchmaker can reach this PC without router port-forward..."
    $sshArgs = @(
        "-N",
        "-o", "BatchMode=yes",
        "-o", "ExitOnForwardFailure=yes",
        "-o", "ServerAliveInterval=30",
        "-o", "StrictHostKeyChecking=accept-new",
        "-o", "IdentitiesOnly=yes",
        "-i", $key,
        "-R", "127.0.0.1:${remotePort}:${local}",
        $script:TunnelSsh
    )
    Start-Process -FilePath "ssh.exe" -ArgumentList $sshArgs -WindowStyle Minimized
    $script:ManualSteps.Add("Leave the minimized ssh window running. If it closes, Azure can no longer start a VM on this PC.")
}

# --- main ---
$script:BootstrapToken = $BootstrapToken
$script:MatchmakerUrl = $MatchmakerUrl
$script:HeadscaleUrl = $HeadscaleUrl
$script:ImageSigningPublicKey = $ImageSigningPublicKey
$script:ImageStandardDigest = $ImageStandardDigest
$script:ImageStandardSignature = $ImageStandardSignature
$script:HostDisplayName = $HostDisplayName
$script:ImageQcow2Url = $ImageQcow2Url
$script:HostAgentInstallerUrl = $HostAgentInstallerUrl
$script:AgentUrlOverride = ""
$script:ListenAddr = ""
$script:TunnelSsh = ""
$script:TunnelRemotePort = ""

if ($PSScriptRoot) {
    Import-SidecarEnv (Join-Path $PSScriptRoot "friend.env")
}
Copy-KitToStableRoot
Import-SidecarEnv (Join-Path $script:StableRoot "friend.env")
Request-Administrator

if ([string]::IsNullOrWhiteSpace($script:BootstrapToken) -or $script:BootstrapToken.Length -lt 16) {
    throw "HOST_BOOTSTRAP_TOKEN is missing. It belongs in friend.env next to setup.cmd (the operator fills this, not you)."
}
if ($script:BootstrapToken -eq $env:HOST_APPROVE_TOKEN) {
    throw "Do not use HOST_APPROVE_TOKEN. This script only accepts the host bootstrap token."
}

Assert-FirmwareVirtualization
Assert-DiskSpace

$dismCode = 0
if (-not $ResumeAfterReboot) {
    $dismCode = Enable-Whpx
}

Install-WebView2
Install-TailscaleClient
Disable-AcSleep

$qemuDir = Install-PinnedQemu
Install-GuestDisk -QemuDir $qemuDir
$agentExe = Install-HostAgentApp

# Default: the Host Agent fetches its commands from the server itself, so
# nothing has to reach this PC from the internet (no port-forward, no
# tunnel). Only a friend.env that sets HOST_AGENT_URL makes the agent listen
# and wait to be dialled.
$listenAddr = ""
$agentUrl = ""
if ($script:AgentUrlOverride) {
    $listenAddr = $script:ListenAddr
    if ([string]::IsNullOrWhiteSpace($listenAddr)) {
        $listenAddr = "0.0.0.0:7842"
    }
    $agentUrl = $script:AgentUrlOverride.Trim().TrimEnd("/")
    Write-Host "Using HOST_AGENT_URL from friend.env (the server dials this origin): $agentUrl"
    # Only this mode listens, so only this mode opens the port.
    Enable-HostFirewall
    Start-ReverseTunnel
}
else {
    Write-Host "This PC will fetch its commands from the server. No router setup is needed."
}

Write-HostAgentEnv -AgentUrl $agentUrl -ListenAddr $listenAddr

if ($dismCode -eq 3010 -and -not $ResumeAfterReboot) {
    Register-ResumeAfterReboot
    Write-Host ""
    Write-Host "Remaining things software cannot do:"
    $script:ManualSteps | ForEach-Object { Write-Host " - $_" }
    Write-Host "Rebooting in 15 seconds. Save your work."
    shutdown.exe /r /t 15 /c "DCL host setup needs a reboot for Windows Hypervisor Platform."
    exit 3010
}

Write-Host "Starting Host Agent. Leave sharing on. Do not log Tailscale into tailscale.com."
if ($agentExe) {
    Start-Process -FilePath $agentExe
}

Write-Host ""
Write-Host "Setup finished."
if ($script:ManualSteps.Count -gt 0) {
    Write-Host "Remaining things software cannot do:"
    $script:ManualSteps | ForEach-Object { Write-Host " - $_" }
}
else {
    Write-Host "No extra steps on this PC. Keep it plugged in and awake. The operator still has to approve the host."
}
