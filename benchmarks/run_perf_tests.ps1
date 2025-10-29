param(
    [string]$ConfigurationPath = "benchmarks/scenarios.json",
    [string]$OutputJsonPath = "benchmarks/results/latest.json",
    [string]$OutputMarkdownPath = "benchmarks/results/latest.md",
    [switch]$Release
)

$buildMode = if ($Release.IsPresent) { "--release" } else { "" }

Write-Host "Running synthetic performance benchmarks..."

$cmd = @(
    "cargo", "run", "-p", "perf_harness",
    $buildMode,
    "--",
    "--config", $ConfigurationPath,
    "--output", $OutputJsonPath,
    "--markdown", $OutputMarkdownPath
) | Where-Object { $_ -ne "" }

& $cmd

if ($LASTEXITCODE -ne 0) {
    Write-Error "Benchmark harness failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "Benchmark results written to" $OutputJsonPath "and" $OutputMarkdownPath
