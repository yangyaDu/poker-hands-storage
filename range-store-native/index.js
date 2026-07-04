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

function fromNativeConcreteLine(line) {
  return {
    concreteLineId: line.concreteLineId,
    abstractLine: line.abstractLine,
    concreteLine: line.concreteLine,
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

  getConcreteLineIdRaw(request) {
    return this.#native.getConcreteLineIdRaw({
      ...toNativeDimension(request),
      concreteLine: request.concreteLine,
    });
  }

  getConcreteLines(request) {
    try {
      return normalizeApiResult(
        this.#native.getConcreteLines({
          ...toNativeDimension(request),
          abstractLine: request.abstractLine,
          concreteLine: request.concreteLine,
        }),
      );
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  getConcreteLinesRaw(request) {
    const result = this.#native.getConcreteLinesRaw({
      ...toNativeDimension(request),
      abstractLine: request.abstractLine,
      concreteLine: request.concreteLine,
    });
    return {
      lines: result.lines.map(fromNativeConcreteLine),
    };
  }

  getAbstractLines(request) {
    try {
      return normalizeApiResult(
        this.#native.getAbstractLines({
          strategy: request.strategy,
          drillName: request.drillName,
          playerCount: request.playerCount,
          drillDepth: request.drillDepth,
        }),
      );
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  getAbstractLinesRaw(request) {
    return this.#native.getAbstractLinesRaw({
      strategy: request.strategy,
      drillName: request.drillName,
      playerCount: request.playerCount,
      drillDepth: request.drillDepth,
    });
  }

  handsByActions(request) {
    try {
      return normalizeApiResult(
        this.#native.handsByActions({
          ...toNativeDimension(request),
          concreteLineId: request.concreteLineId,
          actions: request.actions,
          frequency: request.frequency,
        }),
      );
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  handsByActionsRaw(request) {
    return this.#native.handsByActionsRaw({
      ...toNativeDimension(request),
      concreteLineId: request.concreteLineId,
      actions: request.actions,
      frequency: request.frequency,
    });
  }

  queryHandStrategy(request) {
    try {
      return normalizeApiResult(
        this.#native.queryHandStrategy({
          ...toNativeDimension(request),
          concreteLineId: request.concreteLineId,
          holeCards: request.holeCards,
        }),
      );
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  queryHandStrategyRaw(request) {
    const result = this.#native.queryHandStrategyRaw({
      ...toNativeDimension(request),
      concreteLineId: request.concreteLineId,
      holeCards: request.holeCards,
    });
    return {
      inputHoleCards: result.inputHoleCards,
      handCode: result.handCode,
      actions: result.actions.map(fromNativeAction),
    };
  }

  queryBatch(request) {
    try {
      return normalizeApiResult(
        this.#native.queryBatch({
          ...toNativeDimension(request),
          items: request.items.map((item) => ({
            concreteLineId: item.concreteLineId,
            holeCards: item.holeCards,
          })),
        }),
      );
    } catch (error) {
      return apiErrorResult(error);
    }
  }

  queryBatchRaw(request) {
    const result = this.#native.queryBatchRaw({
      ...toNativeDimension(request),
      items: request.items.map((item) => ({
        concreteLineId: item.concreteLineId,
        holeCards: item.holeCards,
      })),
    });
    return {
      results: result.results.map((item) => ({
        concreteLineId: item.concreteLineId,
        inputHoleCards: item.inputHoleCards,
        actions: item.actions?.map(fromNativeAction),
        error: item.error,
      })),
    };
  }

  prewarm(request) {
    const result = this.#native.prewarm(toNativeDimension(request));
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
