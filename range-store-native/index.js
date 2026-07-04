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

export class PokerHandsRange {
  #native;

  constructor(options) {
    this.#native = new native.PokerHandsRange({
      dataDir: options.dataDir,
      maxOpenHandles: options.maxOpenHandles,
      verifyChecksums: options.verifyChecksums,
    });
  }

  getConcreteLineId(request) {
    return this.#native.getConcreteLineId({
      ...toNativeDimension(request),
      concreteLine: request.concreteLine,
    });
  }

  handsByActions(request) {
    const result = this.#native.handsByActions({
      ...toNativeDimension(request),
      concreteLineId: request.concreteLineId,
      actions: request.actions,
      frequency: request.frequency,
    });
    return { holeCards: result.holeCards };
  }

  queryHandStrategy(request) {
    const result = this.#native.queryHandStrategy({
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
    const result = this.#native.queryBatch({
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
