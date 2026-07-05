import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { performance } from "node:perf_hooks";

const inputPath = process.argv[2];
if (!inputPath) {
  console.error("Missing native benchmark input JSON path");
  process.exit(2);
}

const input = JSON.parse(readFileSync(inputPath, "utf8"));

function memorySnapshot(note) {
  const memory = process.memoryUsage();
  return {
    rssBytes: memory.rss,
    heapTotalBytes: memory.heapTotal,
    heapUsedBytes: memory.heapUsed,
    externalBytes: memory.external,
    arrayBuffersBytes: memory.arrayBuffers,
    note,
  };
}

function safeRatio(numerator, denominator) {
  return denominator === 0 || !Number.isFinite(denominator)
    ? 0
    : numerator / denominator;
}

function percentile(sorted, percentileValue) {
  if (sorted.length === 0) {
    return 0;
  }
  const index = (percentileValue / 100) * (sorted.length - 1);
  const lower = Math.floor(index);
  const upper = Math.ceil(index);
  if (lower === upper) {
    return sorted[lower];
  }
  const fraction = index - lower;
  return sorted[lower] * (1 - fraction) + sorted[upper] * fraction;
}

function measureCase(name, description, items, warmupIterations, operation) {
  const effectiveWarmup = Math.min(items.length, warmupIterations);
  for (let index = 0; index < effectiveWarmup; index += 1) {
    try {
      operation(items[index], index);
    } catch {
      // Warmup errors are ignored to match the Rust benchmark helper.
    }
  }

  let resultCount = 0;
  let errorCount = 0;
  let firstError = null;
  const timesMs = [];
  const caseStart = performance.now();

  for (let index = 0; index < items.length; index += 1) {
    const iterationStart = performance.now();
    try {
      resultCount += operation(items[index], index);
    } catch (error) {
      errorCount += 1;
      if (firstError === null) {
        firstError = error instanceof Error ? error.message : String(error);
      }
    }
    timesMs.push(performance.now() - iterationStart);
  }

  const totalMs = performance.now() - caseStart;
  timesMs.sort((left, right) => left - right);
  return {
    name,
    description,
    iterations: items.length,
    warmupIterations: effectiveWarmup,
    totalMs,
    avgMs: safeRatio(totalMs, items.length),
    p50Ms: percentile(timesMs, 50),
    p90Ms: percentile(timesMs, 90),
    p95Ms: percentile(timesMs, 95),
    p99Ms: percentile(timesMs, 99),
    maxMs: timesMs.length === 0 ? 0 : timesMs[timesMs.length - 1],
    qps: safeRatio(items.length, totalMs / 1000),
    resultCount,
    errorCount,
    firstError,
  };
}

function runWarmupItems(items, warmupIterations, operation) {
  const effectiveWarmup = Math.min(items.length, warmupIterations);
  for (let index = 0; index < effectiveWarmup; index += 1) {
    try {
      operation(items[index], index);
    } catch {
      // Warmup errors are intentionally ignored; measured cases report them.
    }
  }
}

function dimensionRequest(item) {
  return {
    strategy: item.strategy,
    playerCount: item.playerCount,
    depthBb: item.depthBb,
  };
}

function countBatchActions(response) {
  let total = 0;
  for (const item of response.results) {
    if (item.error) {
      throw new Error(item.error);
    }
    total += item.actions?.length ?? 0;
  }
  return total;
}

function requireApiData(response) {
  if (response.code !== 0) {
    throw new Error(response.message ?? `Native API returned code ${response.code}`);
  }
  return response.data;
}

function readApiData(response) {
  if (response && typeof response === "object" && "code" in response) {
    return requireApiData(response);
  }
  return response;
}

function countBatchActionsEnvelope(response) {
  const data = readApiData(response);
  let total = 0;
  for (const item of data.results) {
    if (item.error) {
      throw new Error(item.error.message);
    }
    total += item.actions?.length ?? 0;
  }
  return total;
}

function handsByActionsRequest(item, concreteLineId) {
  const request = {
    ...dimensionRequest(item),
    concreteLineId,
    actions: item.actions,
  };
  if (item.frequency !== null && item.frequency !== undefined) {
    request.frequency = item.frequency;
  }
  return request;
}

function callHandsByActions(mode, store, item, concreteLineId) {
  const request = handsByActionsRequest(item, concreteLineId);
  return readApiData(store.handsByActions(request)).holeCards.length;
}

function callDrillScenario(store, item) {
  return readApiData(
    store.getAbstractLines({
      strategy: item.strategy,
      drillName: item.drillName,
      playerCount: item.playerCount,
      drillDepth: item.drillDepth,
    }),
  ).abstractLines.length;
}

function resolveConcreteLineId(store, item) {
  const data = readApiData(
    store.getConcreteLines({
      ...dimensionRequest(item),
      concreteLine: item.concreteLine,
    }),
  );
  if (data.lines.length !== 1) {
    throw new Error(`expected one concrete line, got ${data.lines.length}`);
  }
  return data.lines[0].concreteLineId;
}

function pushStoreCases(cases, mode, store) {
  const prefix = `native-${mode}`;
  cases.push(
    measureCase(
      `${prefix}:concrete-lines-exact`,
      `Resolve concrete_line through ${prefix} getConcreteLines exact lookup.`,
      input.concreteLineQueries,
      input.warmupIterations,
      (item) => {
        const concreteLineId = resolveConcreteLineId(store, item);
        if (concreteLineId !== item.concreteLineId) {
          throw new Error(
            `concrete line id mismatch: expected ${item.concreteLineId}, got ${concreteLineId}`,
          );
        }
        return 1;
      },
    ),
  );
  cases.push(
    measureCase(
      `${prefix}:hand-strategy`,
      `Single concrete_line_id + hand query through ${prefix} business envelope API.`,
      input.workload.handQueries,
      input.warmupIterations,
      (item) =>
        readApiData(
          store.queryHandStrategy({
            ...dimensionRequest(item),
            concreteLineId: item.concreteLineId,
            holeCards: item.holeCards,
          }),
        ).actions.length,
    ),
  );
  cases.push(
    measureCase(
      `${prefix}:batch-hand-strategy`,
      `Run the default batch-size concrete_line_id + hand lookup case through ${prefix}.`,
      input.workload.batchQueries,
      input.warmupIterations,
      (item) =>
        countBatchActionsEnvelope(
          store.queryBatch({
            ...dimensionRequest(item),
            items: item.requests,
          }),
        ),
    ),
  );

  for (const [size, queries] of input.workload.batchQueriesBySize) {
    cases.push(
      measureCase(
        `${prefix}:batch-size-${size}`,
        `Run ${size} lookups per batch through ${prefix} business envelope API.`,
        queries,
        input.warmupIterations,
        (item) =>
          countBatchActionsEnvelope(
            store.queryBatch({
              ...dimensionRequest(item),
              items: item.requests,
            }),
          ),
      ),
    );
  }

  cases.push(
    measureCase(
      `${prefix}:hands-by-actions`,
      `Decode all hands for one concrete line through ${prefix}.`,
      input.workload.handsByActionsQueries,
      input.warmupIterations,
      (item) => callHandsByActions(mode, store, item, item.concreteLineId),
    ),
  );

  cases.push(
    measureCase(
      `${prefix}:drill-scenarios-metadata`,
      `Read drill scenario abstract lines through ${prefix} getAbstractLines.`,
      input.workload.drillScenarioQueries,
      input.warmupIterations,
      (item) => callDrillScenario(store, item),
    ),
  );

  cases.push(
    measureCase(
      `${prefix}:line-to-hands-by-actions`,
      `Resolve concrete_line and then run handsByActions through ${prefix}.`,
      input.lineToHandsByActionsQueries,
      input.warmupIterations,
      (item) => {
        const concreteLineId = resolveConcreteLineId(store, item);
        if (concreteLineId !== item.concreteLineId) {
          throw new Error(
            `concrete line id mismatch: expected ${item.concreteLineId}, got ${concreteLineId}`,
          );
        }
        return callHandsByActions(mode, store, item, concreteLineId);
      },
    ),
  );
}

function warmupStore(mode, store) {
  runWarmupItems(input.concreteLineQueries, input.warmupIterations, (item) => {
    const concreteLineId = resolveConcreteLineId(store, item);
    if (concreteLineId !== item.concreteLineId) {
      throw new Error(
        `concrete line id mismatch: expected ${item.concreteLineId}, got ${concreteLineId}`,
      );
    }
    return 1;
  });
  runWarmupItems(input.workload.handQueries, input.warmupIterations, (item) =>
    readApiData(
      store.queryHandStrategy({
        ...dimensionRequest(item),
        concreteLineId: item.concreteLineId,
        holeCards: item.holeCards,
      }),
    ).actions.length,
  );
  runWarmupItems(input.workload.batchQueries, input.warmupIterations, (item) =>
    countBatchActionsEnvelope(
      store.queryBatch({
        ...dimensionRequest(item),
        items: item.requests,
      }),
    ),
  );
  for (const [, queries] of input.workload.batchQueriesBySize) {
    runWarmupItems(queries, input.warmupIterations, (item) =>
      countBatchActionsEnvelope(
        store.queryBatch({
          ...dimensionRequest(item),
          items: item.requests,
        }),
      ),
    );
  }
  runWarmupItems(input.workload.handsByActionsQueries, input.warmupIterations, (item) =>
    callHandsByActions(mode, store, item, item.concreteLineId),
  );
  runWarmupItems(input.workload.drillScenarioQueries, input.warmupIterations, (item) =>
    callDrillScenario(store, item),
  );
  runWarmupItems(input.lineToHandsByActionsQueries, input.warmupIterations, (item) => {
    const concreteLineId = resolveConcreteLineId(store, item);
    if (concreteLineId !== item.concreteLineId) {
      throw new Error(
        `concrete line id mismatch: expected ${item.concreteLineId}, got ${concreteLineId}`,
      );
    }
    return callHandsByActions(mode, store, item, concreteLineId);
  });
}

const workerStart = performance.now();
const mode = input.mode;
if (mode !== "sdk") {
  console.error(`Invalid native benchmark mode: ${mode}`);
  process.exit(2);
}

const memoryBefore = memorySnapshot(`Bun process memory before native ${mode} import.`);
const sdkImportStart = performance.now();
const sdkModule = await import(pathToFileURL(input.nativeEntry).href);
const importMs = performance.now() - sdkImportStart;
const StoreClass = sdkModule.PokerHandsRange;

const memoryAfterImport = memorySnapshot(`Bun process memory after native ${mode} import.`);
const constructorStart = performance.now();
const store = new StoreClass({
  dataDir: input.dataDir,
  maxOpenHandles: input.maxOpenHandles,
  verifyChecksums: input.verifyChecksums,
});
const constructorMs = performance.now() - constructorStart;
const memoryAfterConstructor = memorySnapshot(
  `Bun process memory after native ${mode} constructor.`,
);

let firstQueryMs = 0;
let firstQueryResultCount = 0;
let firstQuery = null;
let statsAfterFirstQuery = null;
if (input.workload.handQueries.length > 0) {
  firstQuery = input.workload.handQueries[0];
  const firstQueryStart = performance.now();
  const result = store.queryHandStrategy({
    ...dimensionRequest(firstQuery),
    concreteLineId: firstQuery.concreteLineId,
    holeCards: firstQuery.holeCards,
  });
  firstQueryMs = performance.now() - firstQueryStart;
  firstQueryResultCount = readApiData(result).actions.length;
  statsAfterFirstQuery = readApiData(store.stats());
}
const memoryAfterFirstQuery = memorySnapshot(
  `Bun process memory after native ${mode} first query.`,
);
const warmupStart = performance.now();
warmupStore(mode, store);
const warmupMs = performance.now() - warmupStart;
const memoryAfterWarmup = memorySnapshot(`Bun process memory after native ${mode} warmup.`);

const cases = [];
pushStoreCases(cases, mode, store);

const memoryAfter = memorySnapshot(`Bun process memory after native ${mode} benchmark.`);
const statsAfterBenchmark = readApiData(store.stats());

const output = {
  coldStart: {
    mode: `bun-native-${mode}-worker`,
    importMs,
    constructorMs,
    firstQueryMs,
    warmupMs,
    totalMs: performance.now() - workerStart,
    firstQueryResultCount,
    firstQuery,
    statsAfterFirstQuery,
    memoryAfterImport,
    memoryAfterConstructor,
    memoryAfterFirstQuery,
    memoryAfterWarmup,
  },
  cases,
  memoryBefore,
  memoryAfterImport,
  memoryAfterConstructor,
  memoryAfterWarmup,
  memoryAfter,
  notes: [
    `Native entry: ${input.nativeEntry}`,
    `Native benchmark mode: ${mode}`,
    `Bun runtime: ${Bun.version}`,
    `Native ${mode} stats after benchmark: schemaCount=${statsAfterBenchmark.schemaCount}, openHandleCount=${statsAfterBenchmark.openHandleCount}`,
  ],
};

console.log(JSON.stringify(output));
