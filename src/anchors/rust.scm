; Items the grammar's tags.scm omits but cards anchor (EnvRule::Unverified, a const).
(const_item name: (identifier) @name) @definition.constant

(static_item name: (identifier) @name) @definition.constant

(enum_variant name: (identifier) @name) @definition.constant

(field_declaration name: (field_identifier) @name) @definition.field

; An impl block names the type its methods are qualified by (Router::owner); it defines nothing.
(impl_item type: (type_identifier) @name) @container

(impl_item type: (generic_type type: (type_identifier) @name)) @container
