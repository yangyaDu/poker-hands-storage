[CmdletBinding()]
param(
    [string]$CompactDir = '.tmp\compact-line-matrix-v2-all-dimensions-20260711\default_9max_200BB',
    [string]$CoreDir = 'data\range-strata',
    [string]$Dimension = 'default:9:200',
    [ValidateRange(1, 1000000)]
    [int]$HotIterations = 1000,
    [ValidateRange(0, 1000000)]
    [int]$WarmupIterations = 100,
    [ValidateRange(1, 10000)]
    [int]$ColdRuns = 20,
    [ValidateSet('process-cold', 'os-best-effort', 'linux-drop-cache')]
    [string]$ColdMode = 'process-cold',
    [ValidateRange(1, 1000000)]
    [int]$MaxOpenHandles = 2,
    [switch]$VerifyChecksum,
    [string]$Out = 'reports\benchmark-compact-vs-core.json',
    [string]$Markdown = 'reports\benchmark-compact-vs-core.md'
)

$repoRoot = Split-Path -Parent $PSScriptRoot
function Resolve-RepoPath([string]$Value) {
    if ([IO.Path]::IsPathRooted($Value)) {
        return $Value
    }
    return Join-Path $repoRoot $Value
}

$compactPath = Resolve-RepoPath $CompactDir
$corePath = Resolve-RepoPath $CoreDir
$outPath = Resolve-RepoPath $Out
$markdownPath = Resolve-RepoPath $Markdown
$exe = Join-Path $repoRoot 'target\x86_64-pc-windows-msvc\release\poker-hands-storage-tools.exe'

if (-not (Test-Path -LiteralPath $compactPath -PathType Container)) {
    throw "Compact archive directory does not exist: $compactPath"
}
if (-not (Test-Path -LiteralPath $corePath -PathType Container)) {
    throw "Core data directory does not exist: $corePath"
}
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    & cargo build -p poker-hands-storage-tools --release --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

$arguments = @(
    'benchmark-compact-vs-core',
    '--compact-dir', $compactPath,
    '--core-dir', $corePath,
    '--dimension', $Dimension,
    '--hot-iterations', $HotIterations,
    '--warmup-iterations', $WarmupIterations,
    '--cold-runs', $ColdRuns,
    '--cold-mode', $ColdMode,
    '--max-open-handles', $MaxOpenHandles,
    '--out', $outPath,
    '--md', $markdownPath
)
if ($VerifyChecksum) {
    $arguments += '--verify-checksum'
}

& $exe @arguments
exit $LASTEXITCODE