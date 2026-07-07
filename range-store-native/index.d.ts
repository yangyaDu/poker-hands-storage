/* eslint-disable */
export type RangeStoreErrorCode =
  | "INVALID_ARGUMENT"
  | "DIMENSION_NOT_FOUND"
  | "DATA_FILE_NOT_FOUND"
  | "INVALID_FORMAT"
  | "META_DB_ERROR"
  | "ACTION_SCHEMA_NOT_FOUND"
  | "ABSTRACT_LINE_NOT_FOUND"
  | "CONCRETE_LINE_NOT_FOUND"
  | "HAND_STRATEGY_NOT_FOUND"
  | "DRILL_SCENARIO_NOT_FOUND"
  | "HANDS_NOT_FOUND"
  | "INTERNAL"

export declare class RangeStoreError extends Error {
  name: "RangeStoreError"
  code: RangeStoreErrorCode
}

export interface ActionResult {
  actionName: string
  actionSize: number
  amountBb: number
  frequency: number
  handEv?: number
}

export interface ConcreteLinesRequest {
  strategy?: string
  playerCount: number
  depthBb: number
  abstractLine?: string
  concreteLine?: string
}

export interface ConcreteLineInfo {
  concreteLineId: number
  abstractLine: string
  concreteLine: string
}

export interface ConcreteLinesData {
  lines: Array<ConcreteLineInfo>
}

export interface AbstractLinesRequest {
  strategy?: string
  drillName?: string
  playerCount: number
  drillDepth: number
}

export interface AbstractLinesData {
  abstractLines: Array<string>
}

export interface DimensionInput {
  strategy?: string
  playerCount: number
  depthBb: number
}

export interface HandsByActionsRequest {
  strategy?: string
  playerCount: number
  depthBb: number
  concreteLineId: number
  actions?: Array<string>
  frequency?: number
}

export interface HandsByActionsResponse {
  holeCards: Array<string>
}

export interface PrewarmResponse {
  openHandleCount: number
}

export interface QueryHandStrategyRequest {
  strategy?: string
  playerCount: number
  depthBb: number
  concreteLineId: number
  holeCards: string
}

export interface QueryHandStrategyResponse {
  actions: Array<ActionResult>
}

export interface BatchQueryItem {
  concreteLineId: number
  holeCards: string
}

export interface QueryBatchRequest {
  strategy?: string
  playerCount: number
  depthBb: number
  items: Array<BatchQueryItem>
}

export interface QueryBatchItemResponse {
  concreteLineId: number
  holeCards: string
  actions: Array<ActionResult>
}

export interface QueryBatchResponse {
  results: Array<QueryBatchItemResponse>
}

export interface PokerHandsRangeOptions {
  dataDir: string
  maxOpenHandles?: number
  verifyChecksums?: boolean
}

export declare class PokerHandsRange {
  constructor(options: PokerHandsRangeOptions)
  getConcreteLines(request: ConcreteLinesRequest): ConcreteLinesData
  getAbstractLines(request: AbstractLinesRequest): AbstractLinesData
  handsByActions(request: HandsByActionsRequest): HandsByActionsResponse
  queryHandStrategy(request: QueryHandStrategyRequest): QueryHandStrategyResponse
  queryBatch(request: QueryBatchRequest): QueryBatchResponse
  prewarm(request: DimensionInput): PrewarmResponse
  stats(): StatsResponse
}

export type RangeStoreOptions = PokerHandsRangeOptions

export declare const RangeStore: typeof PokerHandsRange

export declare function getPokerHandsRangeSingleton(
  options: PokerHandsRangeOptions,
): PokerHandsRange

export interface StatsResponse {
  schemaCount: number
  openHandleCount: number
  knownDimensions: Array<string>
}
