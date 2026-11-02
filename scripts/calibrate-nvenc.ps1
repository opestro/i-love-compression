<#
  Finds the NVIDIA (NVENC) quality setting that matches the CPU encoder on this PC.
  Run it on a Windows PC with an NVIDIA GPU, with one of your lecture videos:

    powershell -ExecutionPolicy Bypass -File calibrate-nvenc.ps1 -Video "C:\path\lecture.mp4"

  It encodes a 60 s slice with the app's CPU HEVC settings, then with NVENC at several
  quality (CQ) values, and prints time, size and VMAF for each. Send the table back:
  NVENC_HEVC_CQ / NVENC_AV1_CQ in engine/src/video.rs get set from it.
  The encoder flags below mirror engine/src/video.rs; keep them in sync.
#>
param(
  [Parameter(Mandatory)] [string] $Video,
  [int] $Start = 300,
  [int] $Seconds = 60,
  [string] $FFmpeg = (Join-Path $PSScriptRoot "ffmpeg.exe")
)

if (-not (Test-Path $FFmpeg)) { $FFmpeg = 'ffmpeg' } # fall back to one on PATH
$work = Join-Path $env:TEMP 'video-compresser-calibrate'
New-Item -ItemType Directory -Force $work | Out-Null
$slice = Join-Path $work 'slice.mkv'
& $FFmpeg -hide_banner -loglevel error -y -ss $Start -t $Seconds -i $Video -c copy -an $slice
if ($LASTEXITCODE -ne 0) { throw "Couldn't cut a $Seconds s slice at $Start s from $Video" }

$x265 = @('-c:v', 'libx265', '-preset', 'medium', '-crf', '29.5', '-x265-params', 'rd=4:subme=3:ref=4:log-level=error', '-pix_fmt', 'yuv420p10le')
$nvencCommon = @('-preset', 'p5', '-tune', 'hq', '-rc', 'vbr', '-b:v', '0', '-multipass', 'disabled', '-spatial-aq', '1', '-temporal-aq', '1', '-rc-lookahead', '32', '-pix_fmt', 'p010le')
$hevcNvenc = @('-c:v', 'hevc_nvenc') + $nvencCommon + @('-bf', '4', '-b_ref_mode', 'middle', '-profile:v', 'main10')
$av1Nvenc = @('-c:v', 'av1_nvenc') + $nvencCommon

function Measure-Encode([string] $Name, [string[]] $EncoderArgs) {
  $out = Join-Path $work "$Name.mkv"
  $clock = [Diagnostics.Stopwatch]::StartNew()
  & $FFmpeg -hide_banner -loglevel error -y -hwaccel auto -i $slice -vf fps=15 -g 300 -an @EncoderArgs $out
  $clock.Stop()
  if ($LASTEXITCODE -ne 0) { return [pscustomobject]@{ Encoder = $Name; Seconds = '-'; KB = '-'; VMAF = 'failed' } }
  $log = & $FFmpeg -hide_banner -i $out -i $slice -lavfi '[1:v]fps=15[ref];[0:v][ref]libvmaf=n_threads=8' -f null - 2>&1 | Out-String
  $vmaf = if ($log -match 'VMAF score: ([\d.]+)') { [double]$Matches[1] } else { $null }
  [pscustomobject]@{ Encoder = $Name; Seconds = [math]::Round($clock.Elapsed.TotalSeconds, 1); KB = [math]::Round((Get-Item $out).Length / 1KB); VMAF = $vmaf }
}

Write-Host "Encoding a $Seconds s slice of $Video ..."
$results = @(Measure-Encode 'x265-cpu (app CPU setting)' $x265)
foreach ($cq in 26, 28, 30, 32, 34) { $results += Measure-Encode "hevc_nvenc cq$cq" ($hevcNvenc + @('-cq', "$cq")) }
foreach ($cq in 36, 40, 44) { $results += Measure-Encode "av1_nvenc cq$cq" ($av1Nvenc + @('-cq', "$cq")) } # RTX 40 only
$results | Format-Table -AutoSize

$nvenc = @($results | Where-Object { $_.Encoder -like 'hevc_nvenc*' -and $_.VMAF -is [double] }) # failed rows hold the text 'failed'
$target = $results[0].VMAF - 0.5
$pick = $nvenc | Where-Object { $_.VMAF -ge $target } | Sort-Object KB | Select-Object -First 1
if ($pick) {
  Write-Host ("Suggested HEVC GPU setting: {0} - {1} KB vs {2} KB on the CPU ({3:+0;-0}%), {4}x faster." -f `
    $pick.Encoder, $pick.KB, $results[0].KB, (($pick.KB / $results[0].KB - 1) * 100), [math]::Round($results[0].Seconds / $pick.Seconds, 1))
} elseif ($nvenc.Count -eq 0) {
  Write-Host "NVENC didn't run on this PC; see the ffmpeg errors above (often an NVIDIA driver that's too old)."
} else {
  Write-Host "No NVENC setting reached the CPU quality within 0.5 VMAF; send the table anyway."
}
