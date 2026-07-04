use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use range_store_core::dimension::DimensionRef;
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::query::{RangeStoreError, RangeStoreFacade};

#[napi]
pub struct PokerHandsRange {
    inner: Arc<RangeStoreFacade>,
}

#[napi(object)]
pub struct PokerHandsRangeOptions {
    pub data_dir: String,
    pub max_open_handles: Option<u32>,
    pub verify_checksums: Option<bool>,
}

#[napi(object)]
pub struct DimensionInput {
    pub strategy: Option<String>,
    pub player_count: u32,
    pub depth_bb: u32,
}

#[napi(object)]
pub struct ConcreteLineIdRequest {
    pub strategy: Option<String>,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line: String,
}

#[napi(object)]
pub struct ConcreteLinesRequest {
    pub strategy: Option<String>,
    pub player_count: u32,
    pub depth_bb: u32,
    pub abstract_line: Option<String>,
    pub concrete_line: Option<String>,
}

#[napi(object)]
#[derive(Clone)]
pub struct ConcreteLineInfo {
    pub concrete_line_id: u32,
    pub abstract_line: String,
    pub concrete_line: String,
}

#[napi(object)]
pub struct AbstractLinesRequest {
    pub strategy: Option<String>,
    pub drill_name: Option<String>,
    pub player_count: u32,
    pub drill_depth: u32,
}

#[napi(object)]
pub struct HandsByActionsRequest {
    pub strategy: Option<String>,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_id: u32,
    pub actions: Option<Vec<String>>,
    pub frequency: Option<f64>,
}

#[napi(object)]
pub struct HandsByActionsResponse {
    pub hole_cards: Vec<String>,
}

#[napi(object)]
pub struct QueryHandStrategyRequest {
    pub strategy: Option<String>,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_id: u32,
    pub hole_cards: String,
}

#[napi(object)]
pub struct QueryHandStrategyResponse {
    pub input_hole_cards: String,
    pub hand_code: String,
    pub actions: Vec<ActionResult>,
}

#[napi(object)]
pub struct BatchQueryItem {
    pub concrete_line_id: u32,
    pub hole_cards: String,
}

#[napi(object)]
pub struct QueryBatchRequest {
    pub strategy: Option<String>,
    pub player_count: u32,
    pub depth_bb: u32,
    pub items: Vec<BatchQueryItem>,
}

#[napi(object)]
pub struct QueryBatchResponse {
    pub results: Vec<QueryBatchItemResponse>,
}

#[napi(object)]
pub struct QueryBatchItemResponse {
    pub concrete_line_id: u32,
    pub input_hole_cards: String,
    pub actions: Option<Vec<ActionResult>>,
    pub error: Option<String>,
}

#[napi(object)]
pub struct ActionResult {
    pub action_name: String,
    pub action_size: f64,
    pub amount_bb: f64,
    pub frequency: f64,
    pub hand_ev: Option<f64>,
}

#[napi(object)]
pub struct PrewarmResponse {
    pub open_handle_count: u32,
}

#[napi(object)]
pub struct StatsResponse {
    pub schema_count: u32,
    pub open_handle_count: u32,
    pub known_dimensions: Vec<String>,
}

#[napi]
impl PokerHandsRange {
    #[napi(constructor)]
    pub fn new(options: PokerHandsRangeOptions) -> Result<Self> {
        let max_open_handles = options.max_open_handles.unwrap_or(8).max(1) as usize;
        let verify_checksums = options.verify_checksums.unwrap_or(false);
        let inner = RangeStoreFacade::open(options.data_dir, max_open_handles, verify_checksums)
            .map_err(to_napi_error)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    #[napi]
    pub fn get_concrete_line_id(&self, request: ConcreteLineIdRequest) -> Result<u32> {
        let dimension =
            dimension_from_parts(request.strategy, request.player_count, request.depth_bb);
        self.inner
            .get_concrete_line_id(&dimension, &request.concrete_line)
            .map_err(to_napi_error)
    }

    #[napi]
    pub fn get_concrete_lines(&self, request: ConcreteLinesRequest) -> Result<ConcreteLinesData> {
        let dimension = dimension_from_parts(
            request.strategy.clone(),
            request.player_count,
            request.depth_bb,
        );
        let filter = concrete_line_filter_from_request(&request)?;
        let lines = self
            .inner
            .get_concrete_lines(&dimension, filter)
            .map_err(to_napi_error)?
            .into_iter()
            .map(concrete_line_info_from_core)
            .collect();
        Ok(ConcreteLinesData { lines })
    }

    #[napi]
    pub fn get_abstract_lines(&self, request: AbstractLinesRequest) -> Result<AbstractLinesData> {
        let strategy = request.strategy.unwrap_or_else(|| "default".to_owned());
        let drill_name = request.drill_name.unwrap_or_else(|| "rfi".to_owned());
        let abstract_lines = self
            .inner
            .get_drill_scenario_lines(
                &strategy,
                &drill_name,
                request.player_count,
                request.drill_depth,
            )
            .map_err(to_napi_error)?;
        Ok(AbstractLinesData { abstract_lines })
    }

    #[napi]
    pub fn hands_by_actions(
        &self,
        request: HandsByActionsRequest,
    ) -> Result<HandsByActionsResponse> {
        let dimension =
            dimension_from_parts(request.strategy, request.player_count, request.depth_bb);
        let actions = request.actions.unwrap_or_default();
        let hole_cards = self
            .inner
            .hands_by_action_names(
                &dimension,
                request.concrete_line_id,
                &actions,
                request.frequency,
            )
            .map_err(to_napi_error)?;
        Ok(HandsByActionsResponse { hole_cards })
    }

    #[napi]
    pub fn query_hand_strategy(
        &self,
        request: QueryHandStrategyRequest,
    ) -> Result<QueryHandStrategyResponse> {
        let dimension =
            dimension_from_parts(request.strategy, request.player_count, request.depth_bb);
        let result = self
            .inner
            .query_hand_strategy(&dimension, request.concrete_line_id, &request.hole_cards)
            .map_err(to_napi_error)?;
        Ok(QueryHandStrategyResponse {
            input_hole_cards: result.input_hole_cards,
            hand_code: result.hand_code,
            actions: result
                .actions
                .into_iter()
                .map(action_result_from_core)
                .collect(),
        })
    }

    #[napi]
    pub fn query_batch(&self, request: QueryBatchRequest) -> Result<QueryBatchResponse> {
        let dimension =
            dimension_from_parts(request.strategy, request.player_count, request.depth_bb);
        let requests = request
            .items
            .into_iter()
            .map(|item| (item.concrete_line_id, item.hole_cards))
            .collect::<Vec<_>>();
        let results = self
            .inner
            .query_batch_detailed(&dimension, &requests)
            .map_err(to_napi_error)?
            .into_iter()
            .map(|item| QueryBatchItemResponse {
                concrete_line_id: item.concrete_line_id,
                input_hole_cards: item.hole_cards,
                actions: item
                    .actions
                    .map(|actions| actions.into_iter().map(action_result_from_core).collect()),
                error: item.error.map(|e| e.message),
            })
            .collect();
        Ok(QueryBatchResponse { results })
    }

    #[napi]
    pub fn prewarm(&self, request: DimensionInput) -> Result<PrewarmResponse> {
        let dimension =
            dimension_from_parts(request.strategy, request.player_count, request.depth_bb);
        let open_handle_count = self.inner.prewarm(&dimension).map_err(to_napi_error)?;
        Ok(PrewarmResponse {
            open_handle_count: open_handle_count as u32,
        })
    }

    #[napi]
    pub fn stats(&self) -> StatsResponse {
        StatsResponse {
            schema_count: self.inner.schema_count() as u32,
            open_handle_count: self.inner.open_handle_count() as u32,
            known_dimensions: self.inner.known_dimensions(),
        }
    }
}

#[napi(object)]
pub struct ConcreteLinesData {
    pub lines: Vec<ConcreteLineInfo>,
}

#[napi(object)]
pub struct AbstractLinesData {
    pub abstract_lines: Vec<String>,
}

fn dimension_from_parts(
    strategy: Option<String>,
    player_count: u32,
    depth_bb: u32,
) -> DimensionRef {
    DimensionRef::new(
        strategy.unwrap_or_else(|| "default".to_owned()),
        player_count,
        depth_bb,
    )
}

fn action_result_from_core(action: range_store_core::query::ActionResult) -> ActionResult {
    ActionResult {
        action_name: action.action_name,
        action_size: f64::from(action.action_size),
        amount_bb: f64::from(action.amount_bb),
        frequency: action.frequency,
        hand_ev: action.hand_ev,
    }
}

fn concrete_line_info_from_core(
    row: range_store_core::metadata::ConcreteLineRow,
) -> ConcreteLineInfo {
    ConcreteLineInfo {
        concrete_line_id: row.concrete_line_id,
        abstract_line: row.abstract_line,
        concrete_line: row.concrete_line,
    }
}

fn concrete_line_filter_from_request(
    request: &ConcreteLinesRequest,
) -> Result<ConcreteLineFilter<'_>> {
    match (
        request.abstract_line.as_deref(),
        request.concrete_line.as_deref(),
    ) {
        (Some(abstract_line), Some(concrete_line))
            if !abstract_line.trim().is_empty() && !concrete_line.trim().is_empty() =>
        {
            Ok(ConcreteLineFilter::AbstractAndConcrete {
                abstract_line,
                concrete_line,
            })
        }
        (Some(abstract_line), None) if !abstract_line.trim().is_empty() => {
            Ok(ConcreteLineFilter::Abstract(abstract_line))
        }
        (None, Some(concrete_line)) if !concrete_line.trim().is_empty() => {
            Ok(ConcreteLineFilter::Concrete(concrete_line))
        }
        _ => Err(Error::new(
            Status::InvalidArg,
            "one of abstractLine or concreteLine must be provided and non-empty".to_owned(),
        )),
    }
}

fn to_napi_error(error: RangeStoreError) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("{}:{}: {}", error.code(), error.public_code(), error),
    )
}
