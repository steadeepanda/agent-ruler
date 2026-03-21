//! Runtime-generated OpenCode bridge configuration helpers.
//!
//! This module keeps OpenCode channel bridge settings isolated from other
//! runners so future channel/provider divergence stays localized.

use std::path::PathBuf;

use anyhow::Result;

use crate::config::RuntimeState;
use crate::helpers::channels::telegram_bridge_config::{
    self, TelegramBridgeConfigDefaults, TelegramBridgeConfigPatch,
};
use crate::runners::RunnerKind;

pub use crate::helpers::channels::telegram_bridge_config::{
    TelegramBridgeConfig, TelegramBridgeConfigView,
};

pub const GENERATED_CONFIG_FILE_NAME: &str = "opencode-telegram-channel-bridge.generated.json";
const DEFAULT_STATE_FILE_NAME: &str = "opencode-telegram-state.json";

const DEFAULTS: TelegramBridgeConfigDefaults = TelegramBridgeConfigDefaults {
    generated_config_file_name: GENERATED_CONFIG_FILE_NAME,
    default_state_file_name: DEFAULT_STATE_FILE_NAME,
    runner_kind: RunnerKind::Opencode,
};

/// Resolve generated OpenCode bridge config path.
pub fn generated_config_path(runtime: &RuntimeState) -> PathBuf {
    telegram_bridge_config::generated_config_path(runtime, DEFAULTS)
}

/// Ensure generated OpenCode bridge config exists and is normalized.
pub fn ensure_generated_config(runtime: &RuntimeState) -> Result<TelegramBridgeConfig> {
    telegram_bridge_config::ensure_generated_config(runtime, DEFAULTS)
}

/// Apply operator patch to generated OpenCode bridge config.
pub fn update_generated_config(
    runtime: &RuntimeState,
    patch: &TelegramBridgeConfigPatch,
) -> Result<TelegramBridgeConfig> {
    telegram_bridge_config::update_generated_config(runtime, DEFAULTS, patch)
}
