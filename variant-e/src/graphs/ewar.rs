use super::{GErr, GraphRequest, gerr};
use crate::data::Dataset;
pub fn run(_ds: &Dataset, req: &GraphRequest) -> Result<Vec<(String, Vec<Option<f64>>)>, GErr> {
    Err(gerr("UNKNOWN_GRAPH", format!("{} not implemented yet", req.graph), "/graph"))
}
