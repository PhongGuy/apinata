//! Which masked-field texts show: exactly one `{{name}}` reference and nothing else.

use apinata_app::network_secrets::is_template;
use apinata_app::ui::is_variable_reference;

#[test]
fn a_single_reference_shows() {
    for text in [
        "{{token}}",
        "  {{token}}\t",
        "{{process.env.NAME}}",
        "{{a.b-c_d}}",
        "{{$guid}}",
    ] {
        assert!(is_variable_reference(text), "{text:?}");
    }
}

#[test]
fn anything_else_stays_masked() {
    for text in [
        "",
        "hunter2",
        "lit-{{x}}",
        "{{x}}-lit",
        "{{a}}{{b}}",
        "{{}}",
        "{{ x }}",
        "{{a b}}",
        "{{x}",
        "{x}}",
        "{{{x}}}",
        "}}{{",
    ] {
        assert!(!is_variable_reference(text), "{text:?}");
    }
}

#[test]
fn the_secret_move_rule_is_unchanged() {
    assert!(
        is_template("lit-{{x}}"),
        "secret moves still see mixed text as a template"
    );
}
