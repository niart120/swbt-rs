use swbt::reporting::{Direct, Periodic};
use swbt::{ReportingKind, ReportingMode};

#[test]
fn reporting_modes_project_their_runtime_kind() {
    assert_eq!(Periodic::KIND, ReportingKind::Periodic);
    assert_eq!(Direct::KIND, ReportingKind::Direct);
    assert_eq!(
        ReportingKind::ALL,
        &[ReportingKind::Periodic, ReportingKind::Direct]
    );
}
