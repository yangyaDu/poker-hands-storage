import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, test } from "bun:test";

const nativeAddonPath = fileURLToPath(new URL("../index.node", import.meta.url));
const dataDir = fileURLToPath(new URL("../../data/range-strata", import.meta.url));
const httpUrl = process.env.PHS_HTTP_URL?.replace(/\/$/, "");
const testWithHttp = httpUrl ? test : test.skip;

let PokerHandsRange;

beforeAll(async () => {
  if (!httpUrl) {
    return;
  }
  if (!existsSync(nativeAddonPath)) {
    throw new Error(
      "Missing range-store-native/index.node. Run `bun run build:native` before `bun run test:http-consistency`.",
    );
  }
  if (!existsSync(dataDir)) {
    throw new Error(`Missing SDK consistency fixture data: ${dataDir}`);
  }
  ({ PokerHandsRange } = await import("../index.js"));
});

function openStore() {
  return new PokerHandsRange({
    dataDir,
    maxOpenHandles: 2,
    verifyChecksums: false,
  });
}

function sdkDimension() {
  return {
    strategy: "default",
    playerCount: 6,
    depthBb: 100,
  };
}

function httpDimension() {
  return {
    strategy: "default",
    player_count: 6,
    depth_bb: 100,
  };
}

async function postJson(path, body) {
  const response = await fetch(`${httpUrl}${path}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  const json = await response.json();
  if (!response.ok) {
    throw new Error(`${path} returned HTTP ${response.status}: ${JSON.stringify(json)}`);
  }
  return json;
}

function requireOk(response) {
  expect(response.code).toBe(0);
  expect(response.message ?? null).toBeNull();
  expect(response.data).not.toBeNull();
  return response.data;
}

function normalizeConcreteLines(lines) {
  return lines.map((line) => ({
    concreteLineId: line.concreteLineId ?? line.concrete_line_id,
    abstractLine: line.abstractLine ?? line.abstract_line,
    concreteLine: line.concreteLine ?? line.concrete_line,
  }));
}

function normalizeAction(action) {
  return {
    actionName: action.actionName ?? action.action_name,
    actionSize: action.actionSize ?? action.action_size,
    amountBb: action.amountBb ?? action.amount_bb,
    frequency: action.frequency,
    handEv: action.handEv ?? action.hand_ev ?? null,
  };
}

function normalizeHandStrategy(data) {
  return {
    inputHoleCards: data.inputHoleCards ?? data.input_hole_cards,
    handCode: data.handCode ?? data.hand_code,
    actions: data.actions.map(normalizeAction),
  };
}

function normalizeBatch(data) {
  return data.results.map((item) => ({
    concreteLineId: item.concreteLineId ?? item.concrete_line_id,
    inputHoleCards: item.holeCards ?? item.input_hole_cards,
    actionNames: (item.actions ?? item.strategy?.actions ?? []).map(
      (action) => normalizeAction(action).actionName,
    ),
    errorCode: item.error?.code ?? null,
    errorMessage: item.error?.message ?? null,
  }));
}

function normalizeHandsByActions(data) {
  return [...(data.holeCards ?? data.hands)].sort();
}

describe("Native SDK and HTTP service consistency", () => {
  testWithHttp("matches sampled business endpoints", async () => {
    const store = openStore();

    const sdkConcrete = requireOk(
      store.getConcreteLines({
        ...sdkDimension(),
        concreteLine: "F-F-F",
      }),
    );
    const httpConcrete = requireOk(
      await postJson("/range/concrete-lines", {
        ...httpDimension(),
        concrete_line: "F-F-F",
      }),
    );
    expect(normalizeConcreteLines(sdkConcrete.lines)).toEqual(
      normalizeConcreteLines(httpConcrete.lines),
    );

    const sdkDrill = requireOk(
      store.getAbstractLines({
        strategy: "default",
        drillName: "rfi",
        playerCount: 6,
        drillDepth: 100,
      }),
    );
    const httpDrill = requireOk(
      await postJson("/range/drill-scenarios", {
        strategy: "default",
        drill_name: "rfi",
        player_count: 6,
        drill_depth: 100,
      }),
    );
    expect(sdkDrill.abstractLines).toEqual(httpDrill.abstract_lines);

    const sdkHand = requireOk(
      store.queryHandStrategy({
        ...sdkDimension(),
        concreteLineId: 1,
        holeCards: "AA",
      }),
    );
    const httpHand = requireOk(
      await postJson("/range/hand-strategy", {
        ...httpDimension(),
        concrete_line_id: 1,
        hole_cards: "AA",
      }),
    );
    expect(normalizeHandStrategy(sdkHand)).toEqual(normalizeHandStrategy(httpHand));

    const sdkBatch = requireOk(
      store.queryBatch({
        ...sdkDimension(),
        items: [
          { concreteLineId: 1, holeCards: "AA" },
          { concreteLineId: 1, holeCards: "AsXx" },
        ],
      }),
    );
    const httpBatch = requireOk(
      await postJson("/range/hand-strategy-batch", {
        ...httpDimension(),
        requests: [
          { concrete_line_id: 1, hole_cards: "AA" },
          { concrete_line_id: 1, hole_cards: "AsXx" },
        ],
      }),
    );
    expect(normalizeBatch(sdkBatch)).toEqual(normalizeBatch(httpBatch));

    const sdkHands = requireOk(
      store.handsByActions({
        ...sdkDimension(),
        concreteLineId: 1,
        actions: [],
      }),
    );
    const httpHands = requireOk(
      await postJson("/range/hands-by-actions", {
        ...httpDimension(),
        concrete_line_id: 1,
        actions: [],
      }),
    );
    expect(normalizeHandsByActions(sdkHands)).toEqual(normalizeHandsByActions(httpHands));
  });
});
