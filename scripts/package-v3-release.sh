#!/bin/sh

set -eu

usage() {
  echo "Usage: $0 <release-root> <source-db> <report-dir> <artifact-dir>" >&2
  exit 2
}

[ "$#" -eq 4 ] || usage

release_root=${1%/}
source_db=$2
report_dir=${3%/}
artifact_dir=${4%/}
release_id=$(basename "$release_root")

for command_name in jq tar zstd; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "Missing required command: $command_name" >&2
    exit 1
  }
done

if command -v shasum >/dev/null 2>&1; then
  sha256_file() {
    shasum -a 256 "$1" | awk '{print $1}'
  }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256_file() {
    sha256sum "$1" | awk '{print $1}'
  }
else
  echo "Missing required SHA-256 command: shasum or sha256sum" >&2
  exit 1
fi

[ -d "$release_root" ] || {
  echo "Release root does not exist: $release_root" >&2
  exit 1
}
[ -f "$source_db" ] || {
  echo "Source database does not exist: $source_db" >&2
  exit 1
}
[ -d "$report_dir" ] || {
  echo "Report directory does not exist: $report_dir" >&2
  exit 1
}

dimensions='default_6max_100BB default_6max_200BB default_6max_300BB default_8max_100BB default_8max_200BB default_8max_300BB default_9max_100BB default_9max_200BB default_9max_300BB'
required_files='manifest.json drill-scenarios.pb drill-scenarios.idx abstract-action-paths.pb abstract-action-paths.idx hand-strategies.pb hand-strategies.idx'

dimension_count=0
for dimension in $dimensions; do
  dimension_dir="$release_root/$dimension"
  [ -d "$dimension_dir" ] || {
    echo "Missing release dimension: $dimension_dir" >&2
    exit 1
  }
  dimension_count=$((dimension_count + 1))
  for file_name in $required_files; do
    [ -f "$dimension_dir/$file_name" ] || {
      echo "Missing release file: $dimension_dir/$file_name" >&2
      exit 1
    }
  done

  verify_report="$report_dir/$dimension-verify.json"
  cross_report="$report_dir/$dimension-cross.json"
  benchmark_report="$report_dir/$dimension-benchmark.json"
  jq -e '.ok == true and .failureCount == 0 and .mode == "standalone"' \
    "$verify_report" >/dev/null
  jq -e '.ok == true and .failureCount == 0 and .mode == "sqlite-v3"' \
    "$cross_report" >/dev/null
  jq -e '.correctnessVerified == true' "$benchmark_report" >/dev/null
done

actual_dimension_count=$(find "$release_root" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
[ "$actual_dimension_count" -eq "$dimension_count" ] || {
  echo "Unexpected dimension directory count: expected $dimension_count, found $actual_dimension_count" >&2
  exit 1
}

mkdir -p "$artifact_dir"

checksums_path="$release_root/SHA256SUMS"
payload_bytes=0
if [ -f "$checksums_path" ]; then
  (
    cd "$release_root"
    if command -v shasum >/dev/null 2>&1; then
      shasum -a 256 -c SHA256SUMS >/dev/null
    else
      sha256sum -c SHA256SUMS >/dev/null
    fi
  )
else
  : >"$checksums_path"
  for dimension in $dimensions; do
    for file_name in $required_files; do
      relative_path="$dimension/$file_name"
      checksum=$(sha256_file "$release_root/$relative_path")
      printf '%s  %s\n' "$checksum" "$relative_path" >>"$checksums_path"
    done
  done
fi

for dimension in $dimensions; do
  for file_name in $required_files; do
    file_bytes=$(wc -c <"$release_root/$dimension/$file_name" | tr -d ' ')
    payload_bytes=$((payload_bytes + file_bytes))
  done
done

source_sha256=$(sha256_file "$source_db")
source_bytes=$(wc -c <"$source_db" | tr -d ' ')
payload_file_count=$(wc -l <"$checksums_path" | tr -d ' ')
checksums_sha256=$(sha256_file "$checksums_path")
git_commit=$(git rev-parse HEAD)
created_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

release_manifest="$release_root/RELEASE.json"
if [ -f "$release_manifest" ]; then
  jq -e \
    --arg releaseId "$release_id" \
    --arg sourceSha256 "$source_sha256" \
    --arg gitCommit "$git_commit" \
    --arg checksumsSha256 "$checksums_sha256" \
    '.schemaVersion == 1
      and .releaseId == $releaseId
      and .format == "proto-v3"
      and .source.sha256 == $sourceSha256
      and .build.gitCommit == $gitCommit
      and .payload.checksumsSha256 == $checksumsSha256
      and .evidence.allPassed == true' "$release_manifest" >/dev/null
else
  jq -n \
  --arg releaseId "$release_id" \
  --arg createdAt "$created_at" \
  --arg sourceFile "$(basename "$source_db")" \
  --arg sourceSha256 "$source_sha256" \
  --argjson sourceBytes "$source_bytes" \
  --arg gitCommit "$git_commit" \
  --argjson payloadFileCount "$payload_file_count" \
  --argjson payloadBytes "$payload_bytes" \
  --arg checksumsSha256 "$checksums_sha256" \
  --arg evidenceDirectory "$(basename "$report_dir")" \
  '{
    schemaVersion: 1,
    releaseId: $releaseId,
    format: "proto-v3",
    createdAt: $createdAt,
    source: {
      fileName: $sourceFile,
      byteLength: $sourceBytes,
      sha256: $sourceSha256
    },
    build: {
      gitCommit: $gitCommit,
      tool: "poker-hands-storage-tools"
    },
    dimensions: [
      "default:6:100", "default:6:200", "default:6:300",
      "default:8:100", "default:8:200", "default:8:300",
      "default:9:100", "default:9:200", "default:9:300"
    ],
    payload: {
      fileCount: $payloadFileCount,
      byteLength: $payloadBytes,
      checksumsFile: "SHA256SUMS",
      checksumsSha256: $checksumsSha256
    },
    evidence: {
      directoryName: $evidenceDirectory,
      standaloneReports: 9,
      crossReports: 9,
      benchmarkReports: 9,
      allPassed: true
    }
  }' >"$release_manifest"
fi

verify_summary=$(jq -s '{
  passedDimensions: map(select(.ok == true)) | length,
  failureCount: map(.failureCount) | add,
  elapsedMs: map(.elapsedMs) | add
}' "$report_dir"/*-verify.json)
cross_summary=$(jq -s '{
  passedDimensions: map(select(.ok == true)) | length,
  failureCount: map(.failureCount) | add,
  mappingDifferences: map(.counts.mappingDifferences) | add,
  actionDifferences: map(.counts.actionDifferences) | add,
  cellDifferences: map(.counts.cellDifferences) | add,
  handsVisited: map(.counts.handsVisited) | add,
  actionCellsCompared: map(.counts.actionCellsCompared) | add,
  elapsedMs: map(.elapsedMs) | add
}' "$report_dir"/*-cross.json)
benchmark_summary=$(jq -s '{
  passedDimensions: map(select(.correctnessVerified == true)) | length,
  correctnessVerified: all(.[]; .correctnessVerified == true)
}' "$report_dir"/*-benchmark.json)
per_dimension=$(jq -s 'map({
  dimension,
  metadataP50Ms: .metadataSummary.p50Ms,
  metadataP95Ms: .metadataSummary.p95Ms,
  strategyP50Ms: .strategySummary.p50Ms,
  strategyP95Ms: .strategySummary.p95Ms,
  metadataResidentBytes: .cache.metadata.residentEstimatedBytes,
  strategyResidentBytes: .cache.strategies.residentEstimatedBytes,
  deltaRssBytes: .memory.deltaRssBytes
}) | sort_by(.dimension)' "$report_dir"/*-benchmark.json)

release_summary="$report_dir/release-gate-summary.json"
jq -n \
  --arg releaseId "$release_id" \
  --arg archiveRoot "$release_root" \
  --arg sourceDb "$source_db" \
  --arg sourceSha256 "$source_sha256" \
  --arg gitCommit "$git_commit" \
  --arg releaseManifest "$release_manifest" \
  --argjson dimensions "$dimension_count" \
  --argjson verify "$verify_summary" \
  --argjson crossVerify "$cross_summary" \
  --argjson benchmark "$benchmark_summary" \
  --argjson perDimension "$per_dimension" \
  '{
    releaseId: $releaseId,
    archiveRoot: $archiveRoot,
    sourceDb: $sourceDb,
    sourceSha256: $sourceSha256,
    gitCommit: $gitCommit,
    releaseManifest: $releaseManifest,
    dimensions: $dimensions,
    verify: $verify,
    crossVerify: $crossVerify,
    benchmark: $benchmark,
    perDimension: $perDimension
  }' >"$release_summary"

data_archive="$artifact_dir/poker-hands-v3-$release_id.tar.zst"
evidence_archive="$artifact_dir/poker-hands-v3-$release_id-evidence.tar.zst"
COPYFILE_DISABLE=1 tar -C "$(dirname "$release_root")" -cf - "$release_id" \
  | zstd -19 -T0 -f -o "$data_archive"
COPYFILE_DISABLE=1 tar -C "$(dirname "$report_dir")" \
  --exclude="$(basename "$report_dir")/release-artifacts.json" \
  -cf - "$(basename "$report_dir")" \
  | zstd -19 -T0 -f -o "$evidence_archive"

data_sha256=$(sha256_file "$data_archive")
evidence_sha256=$(sha256_file "$evidence_archive")
data_bytes=$(wc -c <"$data_archive" | tr -d ' ')
evidence_bytes=$(wc -c <"$evidence_archive" | tr -d ' ')
release_manifest_sha256=$(sha256_file "$release_manifest")

artifact_manifest="$artifact_dir/poker-hands-v3-$release_id-artifacts.json"
jq -n \
  --arg releaseId "$release_id" \
  --arg releaseManifest "RELEASE.json" \
  --arg releaseManifestSha256 "$release_manifest_sha256" \
  --arg dataFile "$(basename "$data_archive")" \
  --arg dataSha256 "$data_sha256" \
  --argjson dataBytes "$data_bytes" \
  --arg evidenceFile "$(basename "$evidence_archive")" \
  --arg evidenceSha256 "$evidence_sha256" \
  --argjson evidenceBytes "$evidence_bytes" \
  '{
    schemaVersion: 1,
    releaseId: $releaseId,
    releaseManifest: {
      fileName: $releaseManifest,
      sha256: $releaseManifestSha256
    },
    artifacts: [
      {
        kind: "v3-data",
        fileName: $dataFile,
        byteLength: $dataBytes,
        sha256: $dataSha256
      },
      {
        kind: "release-evidence",
        fileName: $evidenceFile,
        byteLength: $evidenceBytes,
        sha256: $evidenceSha256
      }
    ]
  }' >"$artifact_manifest"
cp "$artifact_manifest" "$report_dir/release-artifacts.json"

printf '%s  %s\n' "$data_sha256" "$(basename "$data_archive")" \
  >"$data_archive.sha256"
printf '%s  %s\n' "$evidence_sha256" "$(basename "$evidence_archive")" \
  >"$evidence_archive.sha256"

echo "Packaged V3 release: $release_id"
echo "  data: $data_archive"
echo "  evidence: $evidence_archive"
echo "  manifest: $artifact_manifest"
