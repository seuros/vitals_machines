use super::*;
use crate::probe::profile::classify;
use crate::probe::{FormFactor, FormFactorEvidence, PowerInfo};

#[test]
fn loader_chassis_names_reuse_the_shared_smbios_classification() {
    for (name, expected) in [
        ("Notebook", FormFactor::Laptop),
        ("Sub Notebook", FormFactor::Laptop),
        ("Convertible", FormFactor::Laptop),
        ("Rack Mount Chassis", FormFactor::Server),
        ("Blade Enclosure", FormFactor::Server),
        ("Low Profile Desktop", FormFactor::Desktop),
        ("Tablet", FormFactor::Tablet),
        ("IoT Gateway", FormFactor::Other),
    ] {
        assert_eq!(
            classify(firmware::chassis_code(name), None, &PowerInfo::default()),
            (expected, FormFactorEvidence::Firmware),
            "{name}"
        );
    }
    for unknown in ["Unknown", "Undefined", "", "future-chassis"] {
        assert_eq!(
            classify(firmware::chassis_code(unknown), None, &PowerInfo::default()),
            (FormFactor::Unknown, FormFactorEvidence::Unknown),
        );
    }
}

#[test]
fn firmware_placeholders_are_not_machine_models() {
    for value in ["", "UNKNOWN", "Default string", "To be filled by O.E.M."] {
        assert_eq!(firmware::model(value), None);
    }
    assert_eq!(
        firmware::model(" HP ProBook 460 "),
        Some("HP ProBook 460".into()),
    );
}
