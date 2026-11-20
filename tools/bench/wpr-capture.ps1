# Records a CPU-sampling ETW trace (with stacks) of one program. Needs an elevated shell:
# the kernel profiler provider only runs as admin. Analyze the .etl with xperf / WPA.
param(
	[Parameter(Mandatory)] [string]$Exe,
	[Parameter(Mandatory)] [string]$Etl,
	[string]$ExeArgs = ''
)
$ErrorActionPreference = 'Stop'
wpr -cancel 2>$null | Out-Null # drop a stale session from an earlier aborted run
wpr -start CPU -filemode
try {
	$extra = if ($ExeArgs) { @{ ArgumentList = $ExeArgs } } else { @{} }
	$p = Start-Process $Exe @extra -WorkingDirectory (Split-Path $Exe) -RedirectStandardOutput "$Etl.stdout.log" -WindowStyle Hidden -PassThru -Wait
	"exit: $($p.ExitCode)" | Set-Content "$Etl.exit.txt"
} finally {
	wpr -stop $Etl
}
