//! C1 core: document model, solver, commands, validation and pure export logic.

mod identity;
mod length;
mod units;

pub use identity::{Label, LabelAllocator, LabelError, PROJECT_NAMESPACE, Uuid};
pub use length::{Length, LengthError};
pub use units::{
    GRID_NM, InputContext, InputRounding, ParseError, ParsedLength, ProjectUnit, echo_length,
    format_length, parse_length,
};
