use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use niri_ipc::{Action, CursorOverlayAlign, CursorOverlaySide, OutputAction};

use crate::utils::version;

#[derive(Parser)]
#[command(author, version = version(), about, long_about = None)]
#[command(args_conflicts_with_subcommands = true)]
#[command(subcommand_value_name = "SUBCOMMAND")]
#[command(subcommand_help_heading = "Subcommands")]
pub struct Cli {
    /// Path to config file (default: `$XDG_CONFIG_HOME/niri/config.kdl`).
    ///
    /// This can also be set with the `NIRI_CONFIG` environment variable. If both are set, the
    /// command line argument takes precedence.
    #[arg(short, long)]
    pub config: Option<PathBuf>,
    /// Import environment globally to systemd and D-Bus, run D-Bus services.
    ///
    /// Set this flag in a systemd service started by your display manager, or when running
    /// manually as your main compositor instance. Do not set when running as a nested window, or
    /// on a TTY as your non-main compositor instance, to avoid messing up the global environment.
    #[arg(long)]
    pub session: bool,
    /// Print whether this binary is the tiri fork.
    #[arg(long)]
    pub is_tiri: bool,
    /// Command to run upon compositor startup.
    #[arg(last = true)]
    pub command: Vec<OsString>,

    #[command(subcommand)]
    pub subcommand: Option<Sub>,
}

#[derive(Subcommand)]
pub enum Sub {
    /// Communicate with the running niri instance.
    Msg {
        #[command(subcommand)]
        msg: Msg,
        /// Format output as JSON.
        #[arg(short, long)]
        json: bool,
    },
    /// Validate the config file.
    Validate {
        /// Path to config file (default: `$XDG_CONFIG_HOME/niri/config.kdl`).
        ///
        /// This can also be set with the `NIRI_CONFIG` environment variable. If both are set, the
        /// command line argument takes precedence.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Cause a panic to check if the backtraces are good.
    Panic,
    /// Generate shell completions.
    Completions { shell: CompletionShell },
}

#[derive(Subcommand)]
pub enum Msg {
    /// List connected outputs.
    Outputs,
    /// List workspaces.
    Workspaces,
    /// List open windows.
    Windows,
    /// List open layer-shell surfaces.
    Layers,
    /// Get the configured keyboard layouts.
    KeyboardLayouts,
    /// Print information about the focused output.
    FocusedOutput,
    /// Print information about the focused window.
    FocusedWindow,
    /// Pick a window with the mouse and print information about it.
    PickWindow,
    /// Pick a color from the screen with the mouse.
    PickColor,
    /// Perform an action.
    Action {
        #[command(subcommand)]
        action: Action,
    },
    /// Change output configuration temporarily.
    ///
    /// The configuration is changed temporarily and not saved into the config file. If the output
    /// configuration subsequently changes in the config file, these temporary changes will be
    /// forgotten.
    Output {
        /// Output name.
        ///
        /// Run `niri msg outputs` to see the output names.
        #[arg()]
        output: String,
        /// Configuration to apply.
        #[command(subcommand)]
        action: OutputAction,
    },
    /// Start continuously receiving events from the compositor.
    EventStream,
    /// Print the version of the running niri instance.
    Version,
    /// Request an error from the running niri instance.
    RequestError,
    /// Print the overview state.
    OverviewState,
    /// List screencasts.
    Casts,
    /// List collaboration remote windows.
    RemoteWindows,
    /// List locally shared collaboration window streams.
    SharedWindowStreams,
    /// List pinned virtual cursors.
    VirtualCursors,
    /// List cursor-anchored overlays.
    CursorOverlays,
    /// Create a pinned virtual cursor.
    CreateVirtualCursor {
        /// Cursor id.
        #[arg(long)]
        cursor_id: String,
        /// Mapped window id.
        #[arg(long)]
        window_id: u64,
        /// Window-relative X coordinate in logical pixels.
        #[arg(long)]
        x: f64,
        /// Window-relative Y coordinate in logical pixels.
        #[arg(long)]
        y: f64,
        /// Built-in cursor shape.
        #[arg(long)]
        shape: Option<niri_ipc::VirtualCursorShape>,
        /// Xcursor theme icon name. Defaults to the normal pointer icon.
        #[arg(long)]
        cursor_icon: Option<String>,
        /// Xcursor theme name. Defaults to the compositor's configured cursor theme.
        #[arg(long)]
        cursor_theme: Option<String>,
        /// Cursor size in logical pixels.
        #[arg(long, default_value_t = 24)]
        size: u16,
        /// Cursor color, as #rgb, #rrggbb, or #rrggbbaa.
        #[arg(long)]
        color: Option<String>,
        /// Cursor outline color, as #rgb, #rrggbb, or #rrggbbaa.
        #[arg(long)]
        outline_color: Option<String>,
        /// Cursor movement duration in milliseconds.
        #[arg(long, default_value_t = 180)]
        duration_ms: u32,
        /// Replace an existing cursor with the same id.
        #[arg(long)]
        replace_existing: bool,
        /// Place the cursor at the current hardware pointer location inside the window.
        #[arg(long)]
        at_pointer: bool,
    },
    /// Temporarily override the rendered hardware pointer cursor.
    SetHardwareCursor {
        /// Xcursor theme name. Defaults to the compositor's configured cursor theme.
        #[arg(long)]
        cursor_theme: Option<String>,
        /// Xcursor theme icon name. Defaults to the normal pointer icon.
        #[arg(long)]
        cursor_icon: Option<String>,
        /// Cursor size in logical pixels.
        #[arg(long)]
        size: Option<u16>,
    },
    /// Clear a temporary rendered hardware pointer cursor override.
    ClearHardwareCursor,
    /// Update a pinned virtual cursor.
    UpdateVirtualCursor {
        /// Cursor id.
        #[arg(long)]
        cursor_id: String,
        /// New mapped window id.
        #[arg(long)]
        window_id: Option<u64>,
        /// New window-relative X coordinate in logical pixels.
        #[arg(long)]
        x: Option<f64>,
        /// New window-relative Y coordinate in logical pixels.
        #[arg(long)]
        y: Option<f64>,
        /// Built-in cursor shape.
        #[arg(long)]
        shape: Option<niri_ipc::VirtualCursorShape>,
        /// Xcursor theme icon name. Use an empty string to reset to the normal pointer icon.
        #[arg(long)]
        cursor_icon: Option<String>,
        /// Xcursor theme name. Use an empty string to reset to the compositor's configured theme.
        #[arg(long)]
        cursor_theme: Option<String>,
        /// Cursor size in logical pixels.
        #[arg(long)]
        size: Option<u16>,
        /// Cursor color, as #rgb, #rrggbb, or #rrggbbaa.
        #[arg(long)]
        color: Option<String>,
        /// Cursor outline color, as #rgb, #rrggbb, or #rrggbbaa.
        #[arg(long)]
        outline_color: Option<String>,
        /// Cursor movement duration in milliseconds.
        #[arg(long)]
        duration_ms: Option<u32>,
        /// Show or hide the cursor.
        #[arg(long)]
        visible: Option<bool>,
        /// Cursor z-index.
        #[arg(long)]
        z_index: Option<i32>,
    },
    /// Destroy a pinned virtual cursor.
    DestroyVirtualCursor {
        /// Cursor id.
        #[arg(long)]
        cursor_id: String,
    },
    /// Register a cursor-anchored layer-shell overlay.
    RegisterCursorOverlay {
        /// Overlay id.
        #[arg(long)]
        overlay_id: String,
        /// Layer-shell namespace to render as overlay content.
        #[arg(long)]
        layer_namespace: String,
        /// Anchor to the hardware pointer.
        #[arg(long, conflicts_with = "anchor_virtual_cursor")]
        anchor_hardware_pointer: bool,
        /// Anchor to this virtual cursor id.
        #[arg(long)]
        anchor_virtual_cursor: Option<String>,
        /// Preferred side relative to the cursor.
        #[arg(long, default_value = "right")]
        side: CursorOverlaySide,
        /// Cross-axis alignment relative to the cursor.
        #[arg(long, default_value = "start")]
        align: CursorOverlayAlign,
        /// Gap between cursor and overlay.
        #[arg(long, default_value_t = 10.)]
        gap: f64,
        /// Extra X offset after side placement.
        #[arg(long, default_value_t = 0.)]
        offset_x: f64,
        /// Extra Y offset after side placement.
        #[arg(long, default_value_t = 0.)]
        offset_y: f64,
        /// Padding from output edges.
        #[arg(long, default_value_t = 8.)]
        edge_padding: f64,
        /// Disable side flipping when the overlay would hit an output edge.
        #[arg(long)]
        no_flip: bool,
        /// Hit-test the overlay at its cursor-anchored visual position.
        #[arg(long)]
        interactive: bool,
        /// Request keyboard focus while the overlay is visible.
        #[arg(long)]
        keyboard_focus: bool,
        /// Replace an existing overlay with the same id.
        #[arg(long)]
        replace_existing: bool,
    },
    /// Update a cursor-anchored layer-shell overlay.
    UpdateCursorOverlay {
        /// Overlay id.
        #[arg(long)]
        overlay_id: String,
        /// New layer-shell namespace to render as overlay content.
        #[arg(long)]
        layer_namespace: Option<String>,
        /// Anchor to the hardware pointer.
        #[arg(long, conflicts_with = "anchor_virtual_cursor")]
        anchor_hardware_pointer: bool,
        /// Anchor to this virtual cursor id.
        #[arg(long)]
        anchor_virtual_cursor: Option<String>,
        /// Preferred side relative to the cursor.
        #[arg(long)]
        side: Option<CursorOverlaySide>,
        /// Cross-axis alignment relative to the cursor.
        #[arg(long)]
        align: Option<CursorOverlayAlign>,
        /// Gap between cursor and overlay.
        #[arg(long)]
        gap: Option<f64>,
        /// Extra X offset after side placement.
        #[arg(long)]
        offset_x: Option<f64>,
        /// Extra Y offset after side placement.
        #[arg(long)]
        offset_y: Option<f64>,
        /// Padding from output edges.
        #[arg(long)]
        edge_padding: Option<f64>,
        /// Enable side flipping when the overlay would hit an output edge.
        #[arg(long, conflicts_with = "no_flip")]
        flip: bool,
        /// Disable side flipping when the overlay would hit an output edge.
        #[arg(long)]
        no_flip: bool,
        /// Show or hide the overlay.
        #[arg(long)]
        visible: Option<bool>,
        /// Enable or disable hit-testing at the cursor-anchored visual position.
        #[arg(long)]
        interactive: Option<bool>,
        /// Enable or disable keyboard focus while the overlay is visible.
        #[arg(long)]
        keyboard_focus: Option<bool>,
        /// Overlay z-index.
        #[arg(long)]
        z_index: Option<i32>,
    },
    /// Unregister a cursor-anchored overlay.
    UnregisterCursorOverlay {
        /// Overlay id.
        #[arg(long)]
        overlay_id: String,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub enum CompletionShell {
    Bash,
    Elvish,
    Fish,
    PowerShell,
    Zsh,
    Nushell,
}

impl TryFrom<CompletionShell> for Shell {
    type Error = &'static str;

    fn try_from(shell: CompletionShell) -> Result<Self, Self::Error> {
        match shell {
            CompletionShell::Bash => Ok(Shell::Bash),
            CompletionShell::Elvish => Ok(Shell::Elvish),
            CompletionShell::Fish => Ok(Shell::Fish),
            CompletionShell::PowerShell => Ok(Shell::PowerShell),
            CompletionShell::Zsh => Ok(Shell::Zsh),
            CompletionShell::Nushell => Err("Nushell should be handled separately"),
        }
    }
}
