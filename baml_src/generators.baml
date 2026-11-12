generator target {
    output_type "rust"
    output_dir "../src"
    version "0.226.3"
    default_client_mode sync
    // The client lives at crate::llm::baml_client, not the crate root the generator assumes, and serde
    // must accept the enum @alias spellings the schema text shows (each alias is its variant in snake_case).
    on_generate #"sed -i 's/^use crate::baml_client/use crate::llm::baml_client/' $(find . -name '*.rs') && sed -i 's/^#\[serde(crate = "::baml::__internal::serde")\]$/&\n#[serde(rename_all = "snake_case")]/' types/enums.rs && rustfmt --edition 2024 mod.rs"#
}
