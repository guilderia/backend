use std::net::{Ipv4Addr, SocketAddr};

use axum::Router;

use guildera_database::DatabaseInfo;
use tokio::net::TcpListener;
use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_scalar::{Scalar, Servable as ScalarServable};

mod api;
pub mod clamav;
pub mod exif;
pub mod metadata;
pub mod mime_type;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    // Configure logging and environment
    guilderia_config::configure!(files);

    // Wait for ClamAV
    clamav::init().await;

    // Configure API schema
    #[derive(OpenApi)]
    #[openapi(
        modifiers(&SecurityAddon),
        paths(
            api::root,
            api::upload_file,
            api::fetch_preview,
            api::fetch_file
        ),
        components(
            schemas(
                revolt_result::Error,
                revolt_result::ErrorType,
                api::RootResponse,
                api::Tag,
                api::UploadPayload,
                api::UploadResponse
            )
        ),
        tags(
            // (name = "Files", description = "File uploads API")
        )
    )]
    struct ApiDoc;

    struct SecurityAddon;

    impl Modify for SecurityAddon {
        fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
            if let Some(components) = openapi.components.as_mut() {
                components.add_security_scheme(
                    "bot_token",
                    SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-Bot-Token"))),
                );
                components.add_security_scheme(
                    "session_token",
                    SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-Session-Token"))),
                );
            }
        }
    }

    // Connect to the database
    let db = DatabaseInfo::Auto.connect().await.unwrap();

    // Configure Axum and router
    let app = Router::new()
        .merge(Scalar::with_url("/scalar", ApiDoc::openapi()))
        .nest("/", api::router().await)
        .with_state(db);

    // Configure TCP listener and bind
    let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 14704));
    let listener = TcpListener::bind(&address).await?;
    axum::serve(listener, app.into_make_service()).await
}
