# Synthetic baseline: runs test suites round-robin for -Seconds, sampling each process
# (CPU, memory, page faults, I/O, threads) and the GPU. Output: one folder per run.
# With -Exe, runs that one program instead and kills it after -Seconds (e.g. kyty_emulator).
param(
	[int]$Seconds = 60,
	[int]$SampleMs = 250,
	[string]$Exe = '',
	[string]$ExeArgs = '',
	[string]$Label = 'app',
	[string]$BinDir = "$PSScriptRoot\..\..\_Build\windows",
	[string]$OutRoot = "$PSScriptRoot\..\..\_Build\bench",
	[string[]]$Suites = @(
		'virtual_memory_allocation', 'memory_tracker', 'page_manager', 'image_page_table', 'lru_cache',
		'archive_file', 'save_data_memory', 'resource_tracking', 'resource_materialization',
		'shader_recompiler_compute', 'png_enc', 'ngs2_sampler', 'ajm_resample')
)
$ErrorActionPreference = 'Stop'
$BinDir = (Resolve-Path $BinDir).Path
$out = Join-Path $OutRoot (Get-Date -Format 'yyyyMMdd-HHmmss')
New-Item -ItemType Directory -Force "$out\stdout" | Out-Null
$cores = [Environment]::ProcessorCount
if ($Exe) { $Suites = @($Label) }

# Environment fingerprint so runs are comparable.
@{
	commit = (git -C $BinDir rev-parse --short HEAD); branch = (git -C $BinDir branch --show-current)
	cpu = (Get-CimInstance Win32_Processor).Name; cores = $cores
	ram_gb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
	gpu = (nvidia-smi --query-gpu=name,driver_version --format=csv,noheader)
	seconds = $Seconds; sample_ms = $SampleMs; suites = $Suites; exe = $Exe; exe_args = $ExeArgs
} | ConvertTo-Json | Set-Content "$out\env.json"

$gpu = Start-Process nvidia-smi -ArgumentList "--query-gpu=timestamp,utilization.gpu,utilization.memory,memory.used,power.draw,clocks.sm --format=csv -lms $SampleMs -f `"$out\gpu.csv`"" -WindowStyle Hidden -PassThru

# Exact per-process totals, readable from the handle after exit (sampling misses short runs).
Add-Type -Namespace Bench -Name Psapi -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)]
public struct Counters {
	public uint cb; public uint PageFaultCount; public UIntPtr PeakWorkingSetSize; public UIntPtr WorkingSetSize;
	public UIntPtr QuotaPeakPagedPoolUsage; public UIntPtr QuotaPagedPoolUsage; public UIntPtr QuotaPeakNonPagedPoolUsage;
	public UIntPtr QuotaNonPagedPoolUsage; public UIntPtr PagefileUsage; public UIntPtr PeakPagefileUsage;
}
[DllImport("psapi.dll", SetLastError = true)]
public static extern bool GetProcessMemoryInfo(IntPtr process, out Counters counters, uint size);
'@
function Get-ExitCounters($handle) {
	$c = New-Object Bench.Psapi+Counters
	$c.cb = [Runtime.InteropServices.Marshal]::SizeOf($c)
	if (-not [Bench.Psapi]::GetProcessMemoryInfo($handle, [ref]$c, $c.cb)) { return $null }
	$c
}

$samples = [System.Collections.Generic.List[object]]::new()
$runs = [System.Collections.Generic.List[object]]::new()
$t0 = [Diagnostics.Stopwatch]::StartNew()
$iter = 0
try {
	while ($t0.Elapsed.TotalSeconds -lt $Seconds) {
		$iter++
		foreach ($s in $Suites) {
			if ($t0.Elapsed.TotalSeconds -ge $Seconds) { break }
			$exePath = if ($Exe) { $Exe } else { Join-Path $BinDir "${s}_tests.exe" }
			if (-not (Test-Path $exePath)) { Write-Warning "missing $exePath"; continue }
			$log = "$out\stdout\$s-$iter.log"
			$extra = if ($ExeArgs) { @{ ArgumentList = $ExeArgs } } else { @{} }
			$p = Start-Process $exePath @extra -WorkingDirectory $BinDir -RedirectStandardOutput $log -RedirectStandardError "$log.err" -WindowStyle Hidden -PassThru
			$h = $p.Handle # keep handle so exit code / times stay readable after exit
			$start = $t0.Elapsed.TotalSeconds
			$last = $null; $maxThreads = 0
			while (-not $p.HasExited) {
				if ($Exe -and $t0.Elapsed.TotalSeconds -ge $Seconds) { Stop-Process -Id $p.Id -Force; break }
				try {
					$p.Refresh()
					$c = Get-CimInstance Win32_Process -Filter "ProcessId=$($p.Id)" -Property PageFaults, ReadTransferCount, WriteTransferCount
					$row = [pscustomobject]@{
						t = [math]::Round($t0.Elapsed.TotalSeconds, 3); suite = $s; iter = $iter
						cpu_s = $p.TotalProcessorTime.TotalSeconds; ws_mb = [math]::Round($p.WorkingSet64 / 1MB, 1)
						private_mb = [math]::Round($p.PrivateMemorySize64 / 1MB, 1); threads = $p.Threads.Count; handles = $p.HandleCount
						page_faults = $c.PageFaults; io_read_mb = [math]::Round($c.ReadTransferCount / 1MB, 2); io_write_mb = [math]::Round($c.WriteTransferCount / 1MB, 2)
					}
					$samples.Add($row); $last = $row
					if ($row.threads -gt $maxThreads) { $maxThreads = $row.threads }
				} catch { } # process exited between HasExited and the read
				Start-Sleep -Milliseconds $SampleMs
			}
			$p.WaitForExit()
			$wall = ($p.ExitTime - $p.StartTime).TotalSeconds # exact, unlike the sampling loop
			$cpu = $p.TotalProcessorTime.TotalSeconds
			$ec = Get-ExitCounters $h
			$runs.Add([pscustomobject]@{
				suite = $s; iter = $iter; exit = $p.ExitCode; wall_s = [math]::Round($wall, 3); cpu_s = [math]::Round($cpu, 3)
				cpu_util_pct = [math]::Round(100 * $cpu / [math]::Max($wall, 0.001) / $cores, 1)
				peak_ws_mb = if ($ec) { [math]::Round($ec.PeakWorkingSetSize.ToUInt64() / 1MB, 1) } else { $null }
				peak_private_mb = if ($ec) { [math]::Round($ec.PeakPagefileUsage.ToUInt64() / 1MB, 1) } else { $null }
				page_faults = if ($ec) { $ec.PageFaultCount } else { $last.page_faults }
				io_read_mb = $last.io_read_mb; io_write_mb = $last.io_write_mb; max_threads = $maxThreads
			})
			Write-Host ("{0,-28} it{1} exit={2} {3,7:N2}s" -f $s, $iter, $p.ExitCode, $wall)
		}
	}
} finally {
	Stop-Process -Id $gpu.Id -ErrorAction SilentlyContinue
	$samples | Export-Csv "$out\samples.csv"
	$runs | Export-Csv "$out\runs.csv"
}
Write-Host "total $([math]::Round($t0.Elapsed.TotalSeconds,1))s -> $out"
