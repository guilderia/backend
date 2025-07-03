use guilderia_okapi::openapi3::SchemaObject;
use guilderia_rocket_okapi::guilderia_okapi::openapi3;
use schemars::schema::Schema;

use crate::Error;

impl guilderia_rocket_okapi::response::OpenApiResponderInner for Error {
    fn responses(
        gen: &mut guilderia_rocket_okapi::gen::OpenApiGenerator,
    ) -> std::result::Result<openapi3::Responses, guilderia_rocket_okapi::OpenApiError> {
        let mut content = guilderia_okapi::Map::new();

        let settings = schemars::gen::SchemaSettings::default().with(|s| {
            s.option_nullable = true;
            s.option_add_null_type = false;
            s.definitions_path = "#/components/schemas/".to_string();
        });

        let mut schema_generator = settings.into_generator();
        let schema = schema_generator.root_schema_for::<Error>();

        let definitions = gen.schema_generator().definitions_mut();
        for (key, value) in schema.definitions {
            definitions.insert(key, value);
        }

        definitions.insert("Error".to_string(), Schema::Object(schema.schema));

        content.insert(
            "application/json".to_string(),
            openapi3::MediaType {
                schema: Some(SchemaObject {
                    reference: Some("#/components/schemas/Error".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        Ok(openapi3::Responses {
            default: Some(openapi3::RefOr::Object(openapi3::Response {
                content,
                description: "An error occurred.".to_string(),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}
