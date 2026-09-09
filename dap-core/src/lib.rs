//! DAP proxy core library.

mod adapter_guard;
mod bounds;
mod capabilities;
mod data_watch;
mod function_resolve;
mod instruction_resolve;
mod parent_watch;
mod child_session;
mod client;
mod control_client;
mod engine;
mod execution_state;
mod gateway;
mod memory;
mod mux_bridge;
mod navigation;
mod proxy;
mod python_routing;
mod registry;
mod render;
mod reverse_request;
mod stack_filter;
mod terminal_style;
mod tree_sitter;
mod router;
mod rust_routing;
mod session;
mod session_init;
mod source_view;
mod stop_policy;

pub use adapter_guard::AdapterGuard;
pub use bounds::{
    DEFAULT_MAX_VALUE_CHARS, DEFAULT_MAX_VARIABLES, truncate_variables_response,
};
pub use capabilities::{AdapterCapabilities, condition_result_is_true};
pub use data_watch::{
    DataWatchEntry, data_watch_should_stop, uses_emulated_data_breakpoints,
};
pub use function_resolve::{find_function_in_dirs, resolve_function_line};
pub use instruction_resolve::{
    instruction_breakpoint_key, resolve_instruction_location,
};
pub use child_session::{
    ChildBackendPlan, ChildSessionConfig, ChildSessionProfile, ChildSessionSpawner, ChildSpawnPlan,
    ChildSpawnResult, ParentBackendKind, ParentSessionContext, StartDebuggingArgs,
    decline_reverse_request, handle_start_debugging, resolve_child_spawn, strip_emit_start_debugging,
};
pub use client::{
    DapClient, ProxyStdioOptions, ProxyStdioSession, initialize_arguments, launch_arguments,
    spawn_proxy_stdio,
};
pub use control_client::{
    ControlClient, ExceptionBreakpointSpec, SourceBreakpointSpec, resolve_control_port,
};
pub use protocol_mux::BREAKPOINT_SNAPSHOT_COMMAND;
pub use engine::DapEngine;
pub use execution_state::{
    ExecutionStateSummary, ExecutionStateTracker, ExecutionStatus, VersionedExecutionState,
};
pub use gateway::{manifest_to_static_plugin, resolve_route, spawn_spec_to_adapter};
pub use memory::{
    DEFAULT_READ_COUNT, MAX_READ_BYTES, MAX_WRITE_BYTES, encode_write_payload,
    format_memory_read, hex_string_to_bytes, parse_address, validate_read_count,
};
pub use mux_bridge::{
    ChildSessionBridge, MultiplexOptions, MultiplexedSessionInfo, RunningMultiplexedProxy,
    connect_control_client, roundtrip_request, run_multiplexed_proxy,
    start_headless_multiplexed_proxy, start_multiplexed_proxy,
};
pub use navigation::{NavigateResult, NavigationType};
pub use render::{
    StackTraceOptions, format_breakpoints, format_breakpoints_table, format_evaluate,
    format_evaluate_pretty, format_exception_breakpoints, format_exception_filters,
    build_sync_presentation, format_data_watches, format_navigate_status, format_scopes,
    format_stack_trace, format_stack_trace_with_options, format_status, format_stop_location,
    format_threads, format_variables, format_variables_table, format_watches,
};
pub use terminal_style::TerminalStyle;
pub use tree_sitter::{
    default_tree_sitter_dir, highlight_snippet, language_is_loaded, language_setup_hint,
    parse_snippet, register_language_from_dir, register_language_static, resident_memory_kib,
    try_load_language, warm_languages_for_colors,
};
pub use parent_watch::{arm_parent_death_signal, parent_went_away, spawn_parent_watch};
pub use reverse_request::handle_reverse_request;
pub use rust_routing::{is_rust_adapter, looks_like_cargo_binary, wants_rust_launch_hints};
pub use python_routing::{
    is_python_adapter, python_launch_arguments, python_program_path, wants_python_launch_hints,
};
pub use proxy::{AdapterSpawnOptions, Backend, default_plugins_dir, kill_adapter_process, load_plugins_from_dir, run_transparent_proxy};
pub use registry::PluginRegistry;
pub use router::{RouteMatch, Router};
pub use session::{LaunchOptions, SessionHandle};
pub use session_init::{
    SessionEntryLocation, SessionInitConfig, SessionInitResult, build_initialize_arguments,
    build_launch_arguments, run_debug_request, run_session_init,
};
pub use source_view::{
    FrameLocation, SourceShowOptions, format_source_show, format_source_show_styled, frame_location,
    read_source_file, read_source_file_with_hints, select_stack_frame,
};
pub use stack_filter::{is_hidden_stack_frame, smart_step_should_skip};
pub use stop_policy::{
    ClientStopPolicy, ensure_navigation_supported, hit_count_matches, path_matches_skip,
    source_paths_match, uses_client_stop_policy, uses_emulated_exception_condition,
    validate_source_line,
};
