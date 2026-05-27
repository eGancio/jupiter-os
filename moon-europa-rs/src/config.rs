use std::path::PathBuf;

/// Moon Europa configuration, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    // Telegram credentials
    pub telegram_api_id: i32,
    pub telegram_api_hash: String,
    pub telegram_folder: Option<String>,
    pub telegram_chats: Vec<String>,

    // Storage
    pub data_dir: PathBuf,
    pub session_path: PathBuf,
    pub onnx_model_dir: PathBuf,

    // Server
    pub mcp_host: String,
    pub mcp_port: u16,
    pub mcp_transport: String,

    // Qdrant
    pub qdrant_url: String,

    // Indexer
    pub reconnect_delay_secs: u64,
    pub batch_size: usize,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, String> {
        let data_dir = PathBuf::from(
            std::env::var("DATA_DIR")
                .or_else(|_| std::env::var("CHROMA_PERSIST_DIR")) // legacy compat
                .unwrap_or_else(|_| "./data".into()),
        );

        let session_path = data_dir.join("telegram");

        let telegram_chats_str =
            std::env::var("TELEGRAM_CHATS").unwrap_or_default();
        let telegram_chats: Vec<String> = telegram_chats_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let telegram_folder = std::env::var("TELEGRAM_FOLDER").ok().filter(|s| !s.is_empty());

        let api_id_str = std::env::var("TELEGRAM_API_ID").unwrap_or_default();
        let telegram_api_id: i32 = api_id_str.parse().unwrap_or(0);

        let onnx_model_dir = data_dir.join("model");

        Ok(Self {
            telegram_api_id,
            telegram_api_hash: std::env::var("TELEGRAM_API_HASH").unwrap_or_default(),
            telegram_folder,
            telegram_chats,
            data_dir: data_dir.clone(),
            session_path,
            onnx_model_dir,
            mcp_host: std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            mcp_port: std::env::var("MCP_PORT")
                .unwrap_or_else(|_| "8200".into())
                .parse()
                .unwrap_or(8200),
            mcp_transport: std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "stdio".into()),
            qdrant_url: std::env::var("QDRANT_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:6334".into()),
            reconnect_delay_secs: 30,
            batch_size: 200,
        })
    }

    /// Check if Telegram is configured (has API credentials).
    pub fn has_telegram(&self) -> bool {
        self.telegram_api_id != 0 && !self.telegram_api_hash.is_empty()
    }
}
