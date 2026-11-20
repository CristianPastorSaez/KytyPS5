# Runs the CI ctest set serially (-j1) and in parallel (-jN) while logging system-wide
# commit, paging, CPU and disk counters, to explain why suites slow down under parallel ctest.
param(
	[int[]]$Jobs = @(1, 12),
	[string]$BuildDir = "$PSScriptRoot\..\..\_Build\windows",
	[string]$OutRoot = "$PSScriptRoot\..\..\_Build\bench"
)
$ErrorActionPreference = 'Stop'
$BuildDir = (Resolve-Path $BuildDir).Path
$out = Join-Path $OutRoot ("ctest-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force $out | Out-Null
$regex = '^(audio_out2_port|ngs2_sampler|pad_haptics|controller_settings|host_input|ime_dialog|lru_cache|resource_tracking|scalar_provenance|resource_materialization|shader_vertex_metadata|archive_file|avplayer_file|virtual_memory_allocation|memory_tracker|page_manager|image_page_table|bit_array|build_flags|event_queue_lifetime|sync_on_address|save_data_memory|trophy_system|audio_out_timing|ajm_resample|png_enc|jpeg_dec|jpeg_enc|occlusion_queries|ces|save_data_dialog|psml)$'
$counters = '\Memory\Committed Bytes', '\Memory\Commit Limit', '\Memory\Available MBytes', '\Memory\Pages/sec',
	'\Memory\Page Faults/sec', '\Processor(_Total)\% Processor Time', '\PhysicalDisk(_Total)\% Disk Time',
	'\Paging File(_Total)\% Usage'
foreach ($j in $Jobs) {
	$perf = Start-Process typeperf -ArgumentList (($counters | ForEach-Object { "`"$_`"" }) + '-si', '1', '-f', 'CSV', '-o', "`"$out\perf-j$j.csv`"", '-y') -WindowStyle Hidden -PassThru
	Start-Sleep 2 # baseline before the run
	$sw = [Diagnostics.Stopwatch]::StartNew()
	ctest --test-dir $BuildDir -j $j -R $regex --output-junit "$out\junit-j$j.xml" *> "$out\ctest-j$j.log"
	$sw.Stop()
	Start-Sleep 2
	Stop-Process -Id $perf.Id -ErrorAction SilentlyContinue
	Write-Host ("-j{0}: {1:N1}s" -f $j, $sw.Elapsed.TotalSeconds)
}
Write-Host "-> $out"
