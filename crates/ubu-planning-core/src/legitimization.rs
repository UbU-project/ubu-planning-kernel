use crate::diagnostics::{Diagnostic, DiagnosticCode};
use crate::response::{Plan, ValidationResult};

pub fn semi_legitimize(_candidate: &Plan) -> ValidationResult {
    ValidationResult::invalid(vec![Diagnostic::new(
        DiagnosticCode::NotYetImplemented,
        "semi-legitimization Legitimizer engine is not yet implemented",
    )])
}

pub fn full_legitimize(_candidate: &Plan) -> ValidationResult {
    ValidationResult::invalid(vec![Diagnostic::new(
        DiagnosticCode::NotYetImplemented,
        "full-legitimization Legitimizer engine is not yet implemented",
    )])
}
