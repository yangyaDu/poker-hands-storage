import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const native = require("./index.node");

function toNativeDimension(request) {
  return {
    strategy: request.strategy,
    playerCount: request.playerCount,
    depthBb: request.depthBb,
  };
}

function fromNativeAction(action) {
  return {
    actionName: action.actionName,
    actionSize: action.actionSize,
    amountBb: action.amountBb,
    frequency: action.frequency,
    handEv: action.handEv,
  };
}

export class RangeStoreError extends Error {
  constructor(code, message, options = undefined) {
    super(message, options);
    this.name = "RangeStoreError";
    this.code = code;
  }
}

function toRangeStoreError(error) {
  const message = error instanceof Error ? error.message : String(error);
  const match = /^RANGE_STORE_ERROR:([A-Z_]+):(.*)$/s.exec(message);
  if (match) {
    return new RangeStoreError(match[1], match[2], { cause: error });
  }
  return new RangeStoreError("INTERNAL", message, { cause: error });
}

function callNative(fn) {
  try {
    return fn();
  } catch (error) {
    throw toRangeStoreError(error);
  }
}

export class PokerHandsRange {
  #native;

  constructor(options) {
    this.#native = callNative(
      () =>
        new native.PokerHandsRange({
          dataDir: options.dataDir,
          maxOpenHandles: options.maxOpenHandles,
          verifyChecksums: options.verifyChecksums,
        }),
    );
  }

  getConcreteLines(request) {
    const result = callNative(() =>
      this.#native.getConcreteLines({
        ...toNativeDimension(request),
        abstractLine: request.abstractLine,
        concreteLine: request.concreteLine,
      }),
    );
    return {
      lines: result.lines.map((line) => ({
        concreteLineId: line.concreteLineId,
        abstractLine: line.abstractLine,
        concreteLine: line.concreteLine,
      })),
    };
  }

  getAbstractLines(request) {
    const result = callNative(() =>
      this.#native.getAbstractLines({
        strategy: request.strategy,
        drillName: request.drillName,
        playerCount: request.playerCount,
        drillDepth: request.drillDepth,
      }),
    );
    return { abstractLines: result.abstractLines };
  }

  handsByActions(request) {
    const result = callNative(() =>
      this.#native.handsByActions({
        ...toNativeDimension(request),
        concreteLineId: request.concreteLineId,
        actions: request.actions,
        frequency: request.frequency,
      }),
    );
    return { holeCards: result.holeCards };
  }

  queryHandStrategy(request) {
    const result = callNative(() =>
      this.#native.queryHandStrategy({
        ...toNativeDimension(request),
        concreteLineId: request.concreteLineId,
        holeCards: request.holeCards,
      }),
    );
    return {
      actions: result.actions.map(fromNativeAction),
    };
  }

  queryBatch(request) {
    const result = callNative(() =>
      this.#native.queryBatch({
        ...toNativeDimension(request),
        items: request.items.map((item) => ({
          concreteLineId: item.concreteLineId,
          holeCards: item.holeCards,
        })),
      }),
    );
    return {
      results: result.results.map((item) => ({
        concreteLineId: item.concreteLineId,
        holeCards: item.holeCards,
        actions: item.actions.map(fromNativeAction),
      })),
    };
  }

  prewarm(request) {
    const result = callNative(() => this.#native.prewarm(toNativeDimension(request)));
    return { openHandleCount: result.openHandleCount };
  }

  stats() {
    const result = this.#native.stats();
    return {
      schemaCount: result.schemaCount,
      openHandleCount: result.openHandleCount,
      knownDimensions: result.knownDimensions,
    };
  }
}

export const RangeStore = PokerHandsRange;

let singletonStore = null;
let singletonOptionsKey = null;

function singletonKey(options) {
  return JSON.stringify({
    dataDir: options.dataDir,
    maxOpenHandles: options.maxOpenHandles ?? null,
    verifyChecksums: options.verifyChecksums ?? null,
  });
}

export function getPokerHandsRangeSingleton(options) {
  const key = singletonKey(options);
  if (singletonStore === null) {
    singletonStore = new PokerHandsRange(options);
    singletonOptionsKey = key;
    return singletonStore;
  }
  if (singletonOptionsKey !== key) {
    throw new Error("PokerHandsRange singleton was already initialized with different options");
  }
  return singletonStore;
}
