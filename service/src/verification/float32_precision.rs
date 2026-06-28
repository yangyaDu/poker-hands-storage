use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Float32CheckReason {
    Ok,
    NonFiniteSource,
    NonFiniteActual,
    Float32ValueMismatch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Float32RoundTripCheck {
    pub ok: bool,
    pub reason: Float32CheckReason,
    pub source_value: f64,
    pub expected_value: f64,
    pub actual_value: f64,
    pub expected_bits: u32,
    pub actual_bits: u32,
    pub quantization_abs_error: f64,
    pub quantization_relative_error: f64,
    pub implementation_abs_error: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullableFloat32CheckReason {
    Ok,
    NullMatch,
    NullMismatch,
    NonFiniteSource,
    NonFiniteActual,
    Float32ValueMismatch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NullableFloat32RoundTripCheck {
    pub ok: bool,
    pub reason: NullableFloat32CheckReason,
    pub value: Option<Float32RoundTripCheck>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Float32ErrorSample {
    pub context: String,
    pub source_value: f64,
    pub expected_value: f64,
    pub actual_value: f64,
    pub expected_bits: String,
    pub actual_bits: String,
    pub quantization_abs_error: f64,
    pub quantization_relative_error: f64,
    pub implementation_abs_error: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Float32PrecisionStats {
    pub checked_values: u64,
    pub null_values: u64,
    pub bit_exact_values: u64,
    pub mismatch_values: u64,
    pub max_quantization_abs_error: f64,
    pub max_quantization_relative_error: f64,
    pub max_implementation_abs_error: f64,
    pub p95_quantization_abs_error: f64,
    pub p99_quantization_abs_error: f64,
    pub top_quantization_errors: Vec<Float32ErrorSample>,
}

pub struct Float32PrecisionStatsAccumulator {
    quantization_sample: Vec<f64>,
    top_errors: Vec<Float32ErrorSample>,
    checked_values: u64,
    null_values: u64,
    bit_exact_values: u64,
    mismatch_values: u64,
    max_quantization_abs_error: f64,
    max_quantization_relative_error: f64,
    max_implementation_abs_error: f64,
    top_error_limit: usize,
    reservoir_size: usize,
    reservoir_state: u32,
}

impl Float32PrecisionStatsAccumulator {
    pub fn new(top_error_limit: usize, reservoir_size: usize) -> Self {
        Self {
            quantization_sample: Vec::new(),
            top_errors: Vec::new(),
            checked_values: 0,
            null_values: 0,
            bit_exact_values: 0,
            mismatch_values: 0,
            max_quantization_abs_error: 0.0,
            max_quantization_relative_error: 0.0,
            max_implementation_abs_error: 0.0,
            top_error_limit,
            reservoir_size,
            reservoir_state: 0x9e37_79b9,
        }
    }

    pub fn add_null(&mut self) {
        self.null_values += 1;
    }

    pub fn add(&mut self, check: Float32RoundTripCheck, context: impl Into<String>) {
        self.checked_values += 1;
        if check.ok {
            self.bit_exact_values += 1;
        } else {
            self.mismatch_values += 1;
        }

        self.max_quantization_abs_error = self
            .max_quantization_abs_error
            .max(check.quantization_abs_error);
        self.max_quantization_relative_error = self
            .max_quantization_relative_error
            .max(check.quantization_relative_error);
        self.max_implementation_abs_error = self
            .max_implementation_abs_error
            .max(check.implementation_abs_error);
        self.add_to_reservoir(check.quantization_abs_error);
        self.add_top_error(check, context.into());
    }

    pub fn to_summary(&self) -> Float32PrecisionStats {
        let mut sorted = self.quantization_sample.clone();
        sorted.sort_by(f64::total_cmp);
        Float32PrecisionStats {
            checked_values: self.checked_values,
            null_values: self.null_values,
            bit_exact_values: self.bit_exact_values,
            mismatch_values: self.mismatch_values,
            max_quantization_abs_error: self.max_quantization_abs_error,
            max_quantization_relative_error: self.max_quantization_relative_error,
            max_implementation_abs_error: self.max_implementation_abs_error,
            p95_quantization_abs_error: percentile(&sorted, 0.95),
            p99_quantization_abs_error: percentile(&sorted, 0.99),
            top_quantization_errors: self.top_errors.clone(),
        }
    }

    fn add_to_reservoir(&mut self, value: f64) {
        if self.reservoir_size == 0 {
            return;
        }
        if self.quantization_sample.len() < self.reservoir_size {
            self.quantization_sample.push(value);
            return;
        }

        self.reservoir_state = self
            .reservoir_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let index = (u64::from(self.reservoir_state) % self.checked_values) as usize;
        if index < self.reservoir_size {
            self.quantization_sample[index] = value;
        }
    }

    fn add_top_error(&mut self, check: Float32RoundTripCheck, context: String) {
        if self.top_error_limit == 0 {
            return;
        }

        self.top_errors.push(Float32ErrorSample {
            context,
            source_value: check.source_value,
            expected_value: check.expected_value,
            actual_value: check.actual_value,
            expected_bits: format_float32_bits(check.expected_bits),
            actual_bits: format_float32_bits(check.actual_bits),
            quantization_abs_error: check.quantization_abs_error,
            quantization_relative_error: check.quantization_relative_error,
            implementation_abs_error: check.implementation_abs_error,
        });
        self.top_errors.sort_by(|left, right| {
            right
                .quantization_abs_error
                .total_cmp(&left.quantization_abs_error)
        });
        if self.top_errors.len() > self.top_error_limit {
            self.top_errors.truncate(self.top_error_limit);
        }
    }
}

impl Default for Float32PrecisionStatsAccumulator {
    fn default() -> Self {
        Self::new(20, 8192)
    }
}

pub fn check_float32_round_trip(source_value: f64, actual_value: f64) -> Float32RoundTripCheck {
    let expected_f32 = source_value as f32;
    let actual_f32 = actual_value as f32;
    let expected_value = expected_f32 as f64;
    let expected_bits = expected_f32.to_bits();
    let actual_bits = actual_f32.to_bits();
    let quantization_abs_error = (source_value - expected_value).abs();
    let quantization_relative_error = quantization_abs_error / source_value.abs().max(1.0);
    let implementation_abs_error = (actual_value - expected_value).abs();

    if !source_value.is_finite() {
        return Float32RoundTripCheck {
            ok: false,
            reason: Float32CheckReason::NonFiniteSource,
            source_value,
            expected_value,
            actual_value,
            expected_bits,
            actual_bits,
            quantization_abs_error,
            quantization_relative_error,
            implementation_abs_error,
        };
    }

    if !actual_value.is_finite() {
        return Float32RoundTripCheck {
            ok: false,
            reason: Float32CheckReason::NonFiniteActual,
            source_value,
            expected_value,
            actual_value,
            expected_bits,
            actual_bits,
            quantization_abs_error,
            quantization_relative_error,
            implementation_abs_error,
        };
    }

    let ok = expected_bits == actual_bits;
    Float32RoundTripCheck {
        ok,
        reason: if ok {
            Float32CheckReason::Ok
        } else {
            Float32CheckReason::Float32ValueMismatch
        },
        source_value,
        expected_value,
        actual_value,
        expected_bits,
        actual_bits,
        quantization_abs_error,
        quantization_relative_error,
        implementation_abs_error,
    }
}

pub fn check_nullable_float32_round_trip(
    source_value: Option<f64>,
    actual_value: Option<f64>,
) -> NullableFloat32RoundTripCheck {
    match (source_value, actual_value) {
        (None, None) => NullableFloat32RoundTripCheck {
            ok: true,
            reason: NullableFloat32CheckReason::NullMatch,
            value: None,
        },
        (None, Some(_)) | (Some(_), None) => NullableFloat32RoundTripCheck {
            ok: false,
            reason: NullableFloat32CheckReason::NullMismatch,
            value: None,
        },
        (Some(source), Some(actual)) => {
            let value = check_float32_round_trip(source, actual);
            NullableFloat32RoundTripCheck {
                ok: value.ok,
                reason: match value.reason {
                    Float32CheckReason::Ok => NullableFloat32CheckReason::Ok,
                    Float32CheckReason::NonFiniteSource => {
                        NullableFloat32CheckReason::NonFiniteSource
                    }
                    Float32CheckReason::NonFiniteActual => {
                        NullableFloat32CheckReason::NonFiniteActual
                    }
                    Float32CheckReason::Float32ValueMismatch => {
                        NullableFloat32CheckReason::Float32ValueMismatch
                    }
                },
                value: Some(value),
            }
        }
    }
}

pub fn format_float32_bits(bits: u32) -> String {
    format!("0x{bits:08x}")
}

fn percentile(sorted_values: &[f64], quantile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let raw_index = (sorted_values.len() as f64 * quantile).ceil() as isize - 1;
    let index = raw_index.clamp(0, sorted_values.len() as isize - 1) as usize;
    sorted_values[index]
}
