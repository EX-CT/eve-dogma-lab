use super::{GErr, GraphRequest, gerr};
use crate::data::Dataset;
#[allow(clippy::type_complexity)]
pub fn run(_ds: &Dataset, req: &GraphRequest) -> Result<(Vec<(String, Vec<Option<f64>>)>, Vec<(String, Vec<Option<u32>>)>), GErr> {
    Err(gerr("UNKNOWN_GRAPH", format!("{} not implemented yet", req.graph), "/graph"))
}
