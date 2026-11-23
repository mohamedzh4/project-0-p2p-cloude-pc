# Pro GPU readiness check for a Windows host. Read-only: changes nothing.
# Run it from an admin PowerShell for the full picture (Hyper-V queries need
# admin); a normal one still reports most of it. Send back the report file.
#
#   powershell -ExecutionPolicy Bypass -File pro-gpu-check.ps1
#
# It answers: can Hyper-V hand one GPU to the dcl-pro-guest VM (DDA or GPU-P)
# while this PC keeps a screen? See docs/runbooks/gate-pro.md.

$ErrorActionPreference = 'Continue'
$report = "pro-gpu-check-$env:COMPUTERNAME-$(Get-Date -Format yyyyMMdd-HHmmss).txt"
Start-Transcript -Path $report | Out-Null

$script:pass = 0; $script:warn = 0; $script:fail = 0
function Ok($m)   { Write-Host "  [ok]   $m"; $script:pass++ }
function Warn($m) { Write-Host "  [warn] $m"; $script:warn++ }
function Bad($m)  { Write-Host "  [FAIL] $m"; $script:fail++ }
function Section($m) { Write-Host ""; Write-Host "== $m" }

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
  [Security.Principal.WindowsBuiltInRole]::Administrator)

Section "Machine"
$os = Get-CimInstance Win32_OperatingSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
Write-Host "  host:    $env:COMPUTERNAME"
Write-Host "  windows: $($os.Caption) build $($os.BuildNumber)"
Write-Host "  cpu:     $($cpu.Name)"
Write-Host "  admin:   $isAdmin"
if (-not $isAdmin) { Warn "Not running as admin: Hyper-V GPU queries below will be skipped" }

Section "Windows edition"
if ($os.Caption -match 'Home') {
  Bad "Windows Home has no Hyper-V: Pro needs Pro, Enterprise, Education or Server"
} elseif ($os.Caption -match 'Server') {
  Ok "Windows Server: DDA (whole-GPU assignment) is supported"
} else {
  Warn "Client Windows: Microsoft supports DDA only on Windows Server. The agent tries GPU-P first here, then DDA (issue 36)"
}

Section "Hyper-V"
$hv = Get-CimInstance Win32_OptionalFeature -Filter "Name='Microsoft-Hyper-V-All'" -ErrorAction SilentlyContinue
if ($hv -and $hv.InstallState -eq 1) { Ok "Hyper-V feature is enabled" }
else { Bad "Hyper-V feature is not enabled (Turn Windows features on or off -> Hyper-V, then reboot)" }
if (Get-Command Get-VM -ErrorAction SilentlyContinue) { Ok "Hyper-V PowerShell module is present" }
else { Bad "Hyper-V PowerShell module missing" }

$cmdlets = 'Dismount-VMHostAssignableDevice','Add-VMAssignableDevice',
           'Get-VMHostPartitionableGpu','Set-VMHostPartitionableGpu',
           'Get-VMPartitionableGpu','Set-VMPartitionableGpu','Add-VMGpuPartitionAdapter'
foreach ($c in $cmdlets) {
  if (Get-Command $c -ErrorAction SilentlyContinue) { Write-Host "  cmdlet $c : present" }
  else { Write-Host "  cmdlet $c : MISSING" }
}
if (-not (Get-Command Get-VMHostPartitionableGpu -ErrorAction SilentlyContinue)) {
  Write-Host "  (Get-VMHostPartitionableGpu is missing; the agent falls back to Get-VMPartitionableGpu)"
}

Section "GPUs"
$gpus = @(Get-CimInstance Win32_VideoController)
$i = 0
foreach ($g in $gpus) {
  $i++
  Write-Host ""
  Write-Host "  [$i] $($g.Name)"
  Write-Host "      status: $($g.Status)  driver: $($g.DriverVersion)"
  Write-Host "      instance: $($g.PNPDeviceID)"
  $loc = (Get-PnpDeviceProperty -InstanceId $g.PNPDeviceID -KeyName DEVPKEY_Device_LocationPaths -ErrorAction SilentlyContinue).Data
  if ($loc) { Write-Host "      location path: $($loc[0])" } else { Write-Host "      location path: none" }
  if ($g.CurrentHorizontalResolution) {
    Write-Host "      drives a screen now: $($g.CurrentHorizontalResolution)x$($g.CurrentVerticalResolution)"
  } else {
    Write-Host "      drives a screen now: no"
  }
}
Write-Host ""
$real = @($gpus | Where-Object { $_.Name -notmatch 'Basic Display|Remote Display|Virtual|Parsec|Meta' })
if ($real.Count -ge 2) { Ok "$($real.Count) GPUs: one can go to the guest, the other keeps the host's screen" }
elseif ($real.Count -eq 1) { Bad "Only one GPU: giving it to the guest leaves this PC with no screen. Turn on the iGPU in the BIOS, or add a second card" }
else { Bad "No GPU found" }

Section "NVIDIA driver (the agent reads UUID and VRAM from it)"
if (Get-Command nvidia-smi -ErrorAction SilentlyContinue) {
  & nvidia-smi -L 2>&1 | ForEach-Object { Write-Host "  $_" }
} else { Write-Host "  nvidia-smi not found (fine for AMD/Intel cards)" }
Write-Host "  Agent builds from 2026-11-23 list every GPU above by PnP instance ID; older builds saw NVIDIA only (issue 35)."

if ($isAdmin -and (Get-Command Get-VM -ErrorAction SilentlyContinue)) {
  Section "Hyper-V GPU partitioning (GPU-P)"
  $p = $null
  if (Get-Command Get-VMHostPartitionableGpu -ErrorAction SilentlyContinue) { $p = Get-VMHostPartitionableGpu -ErrorAction SilentlyContinue }
  elseif (Get-Command Get-VMPartitionableGpu -ErrorAction SilentlyContinue) { $p = Get-VMPartitionableGpu -ErrorAction SilentlyContinue }
  if ($p) {
    $p | ForEach-Object { Write-Host "  $($_.Name)  partitions: $($_.PartitionCount)  valid: $($_.ValidPartitionCounts -join ',')" }
    if (@($p | Where-Object { $_.ValidPartitionCounts -contains 1 }).Count -gt 0) { Ok "A GPU accepts partition count 1 (whole GPU)" }
    else { Warn "No GPU lists 1 in ValidPartitionCounts; the agent needs it" }
  } else { Warn "No partitionable GPU reported" }

  Section "Guest VM"
  $vmName = if ($env:HOST_AGENT_HYPERV_VM) { $env:HOST_AGENT_HYPERV_VM } else { 'dcl-pro-guest' }
  $vm = Get-VM -Name $vmName -ErrorAction SilentlyContinue
  if ($vm) {
    Ok "VM $vmName exists (state $($vm.State), generation $($vm.Generation))"
    $ad = Get-VMAssignableDevice -VMName $vmName -ErrorAction SilentlyContinue
    $gp = Get-VMGpuPartitionAdapter -VMName $vmName -ErrorAction SilentlyContinue
    if ($ad -or $gp) { Warn "VM already has a GPU attached (left over from an earlier run?)" }
  } else { Bad "VM $vmName does not exist; the agent needs it created by hand first" }

  $dismounted = Get-VMHostAssignableDevice -ErrorAction SilentlyContinue
  if ($dismounted) { Warn "Devices already dismounted from the host: $(($dismounted | ForEach-Object LocationPath) -join ', ')" }
}

Section "Summary"
Write-Host "  ok: $script:pass   warnings: $script:warn   failures: $script:fail"
Write-Host "  report saved to: $report"
Stop-Transcript | Out-Null
