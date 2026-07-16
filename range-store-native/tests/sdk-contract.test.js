import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, test } from "bun:test";

const nativeAddonPath = fileURLToPath(new URL("../index.node", import.meta.url));
const dataDir = fileURLToPath(new URL("../../data/proto-v3", import.meta.url));

let PokerHandsRange;
let RangeStoreError;

beforeAll(async () => {
  if (!existsSync(nativeAddonPath)) {
    throw new Error(
      "Missing range-store-native/index.node. Run `bun run build:native` in range-store-native before `bun run test:sdk`.",
    );
  }
  if (!existsSync(dataDir)) {
    throw new Error(`Missing SDK contract fixture data: ${dataDir}`);
  }
  ({ PokerHandsRange, RangeStoreError } = await import("../index.js"));
});

function openStore() {
  return new PokerHandsRange({
    dataDir,
    maxOpenHandles: 2,
    verifyChecksums: false,
  });
}

function baseDimension() {
  return {
    strategy: "default",
    playerCount: 6,
    depthBb: 100,
  };
}

describe("PokerHandsRange SDK contract", () => {
  test("keeps constructor lightweight and lazily warms schema cache on strategy query", () => {
    const store = openStore();

    expect(store.stats()).toMatchObject({
      openHandleCount: 0,
      schemaCount: 0,
    });

    const line = store.getConcreteLines({
      ...baseDimension(),
      concreteLine: "F-F-F",
    });
    expect(line.lines).toHaveLength(1);
    expect(line.lines[0].concreteLineId).toBeGreaterThan(0);
    expect(store.stats().schemaCount).toBe(0);

    const result = store.queryBatch({
      ...baseDimension(),
      items: [
        { concreteLineId: 1, holeCards: "AA" },
        { concreteLineId: 1, holeCards: "KK" },
      ],
    });

    expect(result.results[0]).toMatchObject({
      concreteLineId: 1,
      holeCards: "AA",
    });
    expect(result.results[0].actions.length).toBeGreaterThan(0);
    expect(result.results[0].handCode).toBeUndefined();
    expect(result.results[0].error).toBeUndefined();
    expect(store.stats().schemaCount).toBeGreaterThan(0);
  });

  test("throws RangeStoreError for invalid batch item", () => {
    const store = openStore();

    expect(() =>
      store.queryBatch({
        ...baseDimension(),
        items: [
          { concreteLineId: 1, holeCards: "AA" },
          { concreteLineId: 1, holeCards: "AsXx" },
        ],
      }),
    ).toThrow(RangeStoreError);

    try {
      store.queryBatch({
        ...baseDimension(),
        items: [
          { concreteLineId: 1, holeCards: "AA" },
          { concreteLineId: 1, holeCards: "AsXx" },
        ],
      });
    } catch (error) {
      expect(error).toBeInstanceOf(RangeStoreError);
      expect(error.code).toBe("INVALID_ARGUMENT");
      expect(error.message).toContain("Batch item requests[1] failed");
      expect(error.message).toContain("Invalid card format: AsXx");
      expect(error.message).toContain("from concrete_line_id=1");
      return;
    }
    throw new Error("expected queryBatch to throw");
  });

  test("throws typed not-found errors without converting them to internal errors", () => {
    const store = openStore();

    try {
      store.queryBatch({
        ...baseDimension(),
        items: [{ concreteLineId: 999999999, holeCards: "AA" }],
      });
    } catch (error) {
      expect(error).toBeInstanceOf(RangeStoreError);
      expect(error.code).toBe("CONCRETE_LINE_NOT_FOUND");
      expect(error.message).toContain("concrete_line_id=999999999");
      return;
    }
    throw new Error("expected queryBatch to throw");
  });

  test("handsByActions returns only the documented hands payload", () => {
    const store = openStore();

    const result = store.handsByActions({
      ...baseDimension(),
      concreteLineId: 1,
      actions: [],
    });

    expect(Array.isArray(result.holeCards)).toBe(true);
    expect(result.holeCards.length).toBeGreaterThan(0);
    expect(result.exists).toBeUndefined();
  });
});
