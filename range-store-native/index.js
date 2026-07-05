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

function apiErrorResult(error) {
  const message = error instanceof Error ? error.message : String(error);
  const match = /^([^:]+):(\d+):\s*(.*)$/.exec(message);
  if (match) {
    return {
      code: Number.parseInt(match[2], 10),
      data: null,
      message: match[3] || message,
    };
  }
  return {
    code: 500,
    data: null,
    message,
  };
}

function normalizeApiResult(result) {
  return {
    code: result.code,
    data: result.data ?? null,
    message: result.message ?? null,
  };
}

export class PokerHandsRange {
  #native;

  constructor(options) {
    this.#native = new native.PokerHandsRange({
      dataDir: options.dataDir,
      maxOpenHandles: options.maxOpenHandles,
      verifyChecksums: options.verifyChecksums,
    });
  }

  getConcreteLines(request) {
    try {
      const result = this.#native.getConcreteLines({
        ...toNativeDimension(request),
        abstractLine: request.abstractLine,
        concreteLine: request.concreteLine,
      });
      return normalizeApiResult({
        code: 0,
        data: {
          lines: result.lines.map((line) => ({
            concreteLineId: line.concreteLineId,
            abstractLine: line.abstractLine,
            concreteLine: line.concreteLine,
          })),
        },
        message: null,
      });
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  getAbstractLines(request) {
    try {
      const result = this.#native.getAbstractLines({
        strategy: request.strategy,
        drillName: request.drillName,
        playerCount: request.playerCount,
        drillDepth: request.drillDepth,
      });
      return normalizeApiResult({
        code: 0,
        data: {
          abstractLines: result.abstractLines,
        },
        message: null,
      });
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  handsByActions(request) {
    try {
      const result = this.#native.handsByActions({
        ...toNativeDimension(request),
        concreteLineId: request.concreteLineId,
        actions: request.actions,
        frequency: request.frequency,
      });
      return normalizeApiResult({
        code: 0,
        data: {
          holeCards: result.holeCards,
        },
        message: null,
      });
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  queryHandStrategy(request) {
    try {
      const result = this.#native.queryHandStrategy({
        ...toNativeDimension(request),
        concreteLineId: request.concreteLineId,
        holeCards: request.holeCards,
      });
      return normalizeApiResult({
        code: 0,
        data: {
          inputHoleCards: result.inputHoleCards,
          handCode: result.handCode,
          actions: result.actions.map(fromNativeAction),
        },
        message: null,
      });
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  queryBatch(request) {
    try {
      const result = this.#native.queryBatch({
        ...toNativeDimension(request),
        items: request.items.map((item) => ({
          concreteLineId: item.concreteLineId,
          holeCards: item.holeCards,
        })),
      });
      return normalizeApiResult({
        code: 0,
        data: {
          results: result.results.map((item) => ({
            concreteLineId: item.concreteLineId,
            holeCards: item.inputHoleCards,
            actions: item.actions?.map(fromNativeAction),
            error: item.error
              ? { code: item.error.code, message: item.error.message }
              : undefined,
          })),
        },
        message: null,
      });
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  prewarm(request) {
    const result = this.#native.prewarm(toNativeDimension(request));
    return {
      code: 0,
      data: { openHandleCount: result.openHandleCount },
      message: null,
    };
  }

  stats() {
    const result = this.#native.stats();
    return {
      code: 0,
      data: {
        schemaCount: result.schemaCount,
        openHandleCount: result.openHandleCount,
        knownDimensions: result.knownDimensions,
      },
      message: null,
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
