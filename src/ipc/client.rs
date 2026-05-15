use std::io::ErrorKind;
use std::iter::Peekable;
use std::path::Path;
use std::{env, slice};

use anyhow::{anyhow, bail, Context};
use niri_config::OutputName;
use niri_ipc::socket::Socket;
use niri_ipc::{
    Action, Cast, CastKind, CastTarget, CursorOverlayAnchor, CursorOverlayPlacement, Event,
    HardwareCursorOverride, KeyboardLayouts, LogicalOutput, Mode, Output, OutputConfigChanged,
    Overview, Request, Response, RgbaColor, Transform, VirtualCursorAnimation,
    VirtualCursorAppearance, VirtualCursorCreate, VirtualCursorCurve, VirtualCursorSource,
    VirtualCursorUpdate, Window, WindowLayout,
};
use serde_json::json;

use crate::cli::Msg;
use crate::utils::version;

pub fn handle_msg(mut msg: Msg, json: bool) -> anyhow::Result<()> {
    // For actions taking paths, prepend the niri CLI's working directory.
    if let Msg::Action {
        action:
            Action::Screenshot { path, .. }
            | Action::ScreenshotScreen { path, .. }
            | Action::ScreenshotWindow { path, .. }
            | Action::CuaScreenshotWindow { path, .. }
            | Action::CuaScreenshotWorkspace { path, .. },
    } = &mut msg
    {
        if let Some(path) = path {
            ensure_absolute_path(path).context("error making the path absolute")?;
        }
    }

    let request = match &msg {
        Msg::Version => Request::Version,
        Msg::Outputs => Request::Outputs,
        Msg::FocusedWindow => Request::FocusedWindow,
        Msg::FocusedOutput => Request::FocusedOutput,
        Msg::PickWindow => Request::PickWindow,
        Msg::PickColor => Request::PickColor,
        Msg::Action { action } => Request::Action(action.clone()),
        Msg::Output { output, action } => Request::Output {
            output: output.clone(),
            action: action.clone(),
        },
        Msg::Workspaces => Request::Workspaces,
        Msg::Windows => Request::Windows,
        Msg::Layers => Request::Layers,
        Msg::KeyboardLayouts => Request::KeyboardLayouts,
        Msg::EventStream => Request::EventStream,
        Msg::RequestError => Request::ReturnError,
        Msg::OverviewState => Request::OverviewState,
        Msg::Casts => Request::Casts,
        Msg::RemoteWindows => Request::RemoteWindows,
        Msg::SharedWindowStreams => Request::SharedWindowStreams,
        Msg::VirtualCursors => Request::VirtualCursors,
        Msg::CursorOverlays => Request::CursorOverlays,
        Msg::CreateVirtualCursor {
            cursor_id,
            window_id,
            x,
            y,
            shape,
            cursor_icon,
            cursor_theme,
            size,
            color,
            outline_color,
            duration_ms,
            replace_existing,
            at_pointer,
        } => Request::CreateVirtualCursor {
            cursor: VirtualCursorCreate {
                cursor_id: cursor_id.clone(),
                window_id: *window_id,
                x: *x,
                y: *y,
                appearance: Some(VirtualCursorAppearance {
                    source: virtual_cursor_source(
                        *shape,
                        cursor_theme.clone(),
                        cursor_icon.clone(),
                    ),
                    shape: (*shape).unwrap_or_default(),
                    size: *size,
                    color: parse_rgba_color_opt(color.as_deref())?.unwrap_or(RgbaColor {
                        r: 0.18,
                        g: 0.83,
                        b: 0.75,
                        a: 0.95,
                    }),
                    outline_color: parse_rgba_color_opt(outline_color.as_deref())?.unwrap_or(
                        RgbaColor {
                            r: 0.02,
                            g: 0.03,
                            b: 0.04,
                            a: 0.85,
                        },
                    ),
                    opacity: 1.,
                }),
                animation: Some(VirtualCursorAnimation {
                    duration_ms: *duration_ms,
                    curve: VirtualCursorCurve::EaseOutCubic,
                }),
                visible: Some(true),
                z_index: Some(0),
                replace_existing: *replace_existing,
                at_pointer: *at_pointer,
            },
        },
        Msg::SetHardwareCursor {
            cursor_theme,
            cursor_icon,
            size,
        } => Request::SetHardwareCursor {
            cursor: HardwareCursorOverride {
                theme: cursor_theme.clone().and_then(non_empty_string),
                icon: cursor_icon.clone().and_then(non_empty_string),
                size: *size,
            },
        },
        Msg::ClearHardwareCursor => Request::ClearHardwareCursor,
        Msg::UpdateVirtualCursor {
            cursor_id,
            window_id,
            x,
            y,
            shape,
            cursor_icon,
            cursor_theme,
            size,
            color,
            outline_color,
            duration_ms,
            visible,
            z_index,
        } => Request::UpdateVirtualCursor {
            cursor: VirtualCursorUpdate {
                cursor_id: cursor_id.clone(),
                window_id: *window_id,
                x: *x,
                y: *y,
                appearance: if shape.is_some()
                    || cursor_icon.is_some()
                    || cursor_theme.is_some()
                    || size.is_some()
                    || color.is_some()
                    || outline_color.is_some()
                {
                    Some(VirtualCursorAppearance {
                        source: virtual_cursor_source(
                            *shape,
                            cursor_theme.clone(),
                            cursor_icon.clone(),
                        ),
                        shape: (*shape).unwrap_or(niri_ipc::VirtualCursorShape::Ring),
                        size: (*size).unwrap_or(24),
                        color: parse_rgba_color_opt(color.as_deref())?.unwrap_or(RgbaColor {
                            r: 0.18,
                            g: 0.83,
                            b: 0.75,
                            a: 0.95,
                        }),
                        outline_color: parse_rgba_color_opt(outline_color.as_deref())?.unwrap_or(
                            RgbaColor {
                                r: 0.02,
                                g: 0.03,
                                b: 0.04,
                                a: 0.85,
                            },
                        ),
                        opacity: 1.,
                    })
                } else {
                    None
                },
                animation: (*duration_ms).map(|duration_ms| VirtualCursorAnimation {
                    duration_ms,
                    curve: VirtualCursorCurve::EaseOutCubic,
                }),
                visible: *visible,
                z_index: *z_index,
            },
        },
        Msg::DestroyVirtualCursor { cursor_id } => Request::DestroyVirtualCursor {
            cursor_id: cursor_id.clone(),
        },
        Msg::RegisterCursorOverlay {
            overlay_id,
            layer_namespace,
            anchor_hardware_pointer,
            anchor_virtual_cursor,
            side,
            align,
            gap,
            offset_x,
            offset_y,
            edge_padding,
            no_flip,
            interactive,
            keyboard_focus,
            replace_existing,
        } => Request::RegisterCursorOverlay {
            overlay: niri_ipc::CursorOverlayRegister {
                overlay_id: overlay_id.clone(),
                layer_namespace: layer_namespace.clone(),
                anchor: cursor_overlay_anchor(*anchor_hardware_pointer, anchor_virtual_cursor)?,
                placement: CursorOverlayPlacement {
                    side: *side,
                    align: *align,
                    gap: *gap,
                    offset_x: *offset_x,
                    offset_y: *offset_y,
                    edge_padding: *edge_padding,
                    flip: !*no_flip,
                },
                visible: Some(true),
                interactive: Some(*interactive),
                keyboard_focus: Some(*keyboard_focus),
                z_index: Some(0),
                replace_existing: *replace_existing,
            },
        },
        Msg::UpdateCursorOverlay {
            overlay_id,
            layer_namespace,
            anchor_hardware_pointer,
            anchor_virtual_cursor,
            side,
            align,
            gap,
            offset_x,
            offset_y,
            edge_padding,
            flip,
            no_flip,
            visible,
            interactive,
            keyboard_focus,
            z_index,
        } => Request::UpdateCursorOverlay {
            overlay: niri_ipc::CursorOverlayUpdate {
                overlay_id: overlay_id.clone(),
                layer_namespace: layer_namespace.clone(),
                anchor: cursor_overlay_anchor_opt(*anchor_hardware_pointer, anchor_virtual_cursor)?,
                placement: cursor_overlay_placement_opt(
                    *side,
                    *align,
                    *gap,
                    *offset_x,
                    *offset_y,
                    *edge_padding,
                    *flip,
                    *no_flip,
                ),
                visible: *visible,
                interactive: *interactive,
                keyboard_focus: *keyboard_focus,
                z_index: *z_index,
            },
        },
        Msg::UnregisterCursorOverlay { overlay_id } => Request::UnregisterCursorOverlay {
            overlay_id: overlay_id.clone(),
        },
    };

    let mut socket = Socket::connect().context("error connecting to the niri socket")?;

    let result = socket.send(request);

    // For errors that can be caused by a version mismatch between the running niri instance and
    // the niri msg CLI, we will try to fetch and compare the versions.
    let check_compositor_version = match &result {
        Err(err) => {
            // Response JSON parsing errors.
            matches!(
                err.kind(),
                ErrorKind::InvalidData | ErrorKind::UnexpectedEof
            )
        }
        // Error returned from niri.
        Ok(Err(_)) => true,
        _ => false,
    };

    let compositor_version = if check_compositor_version && !matches!(msg, Msg::Version) {
        // Reconnect to support older niri versions with one request per connection.
        Socket::connect()
            .and_then(|mut socket| socket.send(Request::Version))
            .ok()
    } else {
        None
    };

    // Default SIGPIPE so that our prints don't panic on stdout closing.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    // Check for CLI-server version mismatch to add helpful context.
    match compositor_version {
        Some(Ok(Response::Version(compositor_version))) => {
            let cli_version = version();
            if cli_version != compositor_version {
                eprintln!("Running niri compositor has a different version from the niri CLI:");
                eprintln!("Compositor version: {compositor_version}");
                eprintln!("CLI version:        {cli_version}");
                eprintln!("Did you forget to restart niri after an update?");
                eprintln!();
            }
        }
        Some(_) => {
            eprintln!("Unable to get the running niri compositor version.");
            eprintln!("Did you forget to restart niri after an update?");
            eprintln!();
        }
        None => {
            // Communication error, or the original request was already a version request, or the
            // original request had succeeded. Don't add irrelevant context.
        }
    }

    let reply = result.context("error communicating with niri")?;
    let response = reply.map_err(|err_msg| anyhow!(err_msg).context("niri returned an error"))?;

    match msg {
        Msg::RequestError => {
            bail!("unexpected response: expected an error, got {response:?}");
        }
        Msg::Version => {
            let Response::Version(compositor_version) = response else {
                bail!("unexpected response: expected Version, got {response:?}");
            };

            let cli_version = version();

            if json {
                println!(
                    "{}",
                    json!({
                        "compositor": compositor_version,
                        "cli": cli_version,
                    })
                );
                return Ok(());
            }

            if cli_version != compositor_version {
                eprintln!("Running niri compositor has a different version from the niri CLI.");
                eprintln!("Did you forget to restart niri after an update?");
                eprintln!();
            }

            println!("Compositor version: {compositor_version}");
            println!("CLI version:        {cli_version}");
        }
        Msg::Outputs => {
            let Response::Outputs(outputs) = response else {
                bail!("unexpected response: expected Outputs, got {response:?}");
            };

            if json {
                let output =
                    serde_json::to_string(&outputs).context("error formatting response")?;
                println!("{output}");
                return Ok(());
            }

            let mut outputs = outputs
                .into_values()
                .map(|out| (OutputName::from_ipc_output(&out), out))
                .collect::<Vec<_>>();
            outputs.sort_unstable_by(|a, b| a.0.compare(&b.0));

            for (_name, output) in outputs.into_iter() {
                print_output(output)?;
                println!();
            }
        }
        Msg::FocusedWindow => {
            let Response::FocusedWindow(window) = response else {
                bail!("unexpected response: expected FocusedWindow, got {response:?}");
            };

            if json {
                let window = serde_json::to_string(&window).context("error formatting response")?;
                println!("{window}");
                return Ok(());
            }

            if let Some(window) = window {
                print_window(&window);
            } else {
                println!("No window is focused.");
            }
        }
        Msg::Windows => {
            let Response::Windows(mut windows) = response else {
                bail!("unexpected response: expected Windows, got {response:?}");
            };

            if json {
                let windows =
                    serde_json::to_string(&windows).context("error formatting response")?;
                println!("{windows}");
                return Ok(());
            }

            windows.sort_unstable_by_key(|a| a.id);

            for window in windows {
                print_window(&window);
                println!();
            }
        }
        Msg::Layers => {
            let Response::Layers(mut layers) = response else {
                bail!("unexpected response: expected Layers, got {response:?}");
            };

            if json {
                let layers = serde_json::to_string(&layers).context("error formatting response")?;
                println!("{layers}");
                return Ok(());
            }

            layers.sort_by(|a, b| {
                Ord::cmp(&a.output, &b.output)
                    .then_with(|| Ord::cmp(&a.layer, &b.layer))
                    .then_with(|| Ord::cmp(&a.namespace, &b.namespace))
            });
            let mut iter = layers.iter().peekable();

            let print = |surface: &niri_ipc::LayerSurface| {
                println!("    Surface:");
                println!("      Namespace: \"{}\"", &surface.namespace);

                let interactivity = match surface.keyboard_interactivity {
                    niri_ipc::LayerSurfaceKeyboardInteractivity::None => "none",
                    niri_ipc::LayerSurfaceKeyboardInteractivity::Exclusive => "exclusive",
                    niri_ipc::LayerSurfaceKeyboardInteractivity::OnDemand => "on-demand",
                };
                println!("      Keyboard interactivity: {interactivity}");
            };

            let print_layer = |iter: &mut Peekable<slice::Iter<niri_ipc::LayerSurface>>,
                               output: &str,
                               layer| {
                let mut empty = true;
                while let Some(surface) = iter.next_if(|s| s.output == output && s.layer == layer) {
                    empty = false;
                    println!();
                    print(surface);
                }
                if empty {
                    println!(" (empty)\n");
                } else {
                    println!();
                }
            };

            while let Some(surface) = iter.peek() {
                let output = &surface.output;
                println!("Output \"{output}\":");

                print!("  Background layer:");
                print_layer(&mut iter, output, niri_ipc::Layer::Background);

                print!("  Bottom layer:");
                print_layer(&mut iter, output, niri_ipc::Layer::Bottom);

                print!("  Top layer:");
                print_layer(&mut iter, output, niri_ipc::Layer::Top);

                print!("  Overlay layer:");
                print_layer(&mut iter, output, niri_ipc::Layer::Overlay);
            }
        }
        Msg::FocusedOutput => {
            let Response::FocusedOutput(output) = response else {
                bail!("unexpected response: expected FocusedOutput, got {response:?}");
            };

            if json {
                let output = serde_json::to_string(&output).context("error formatting response")?;
                println!("{output}");
                return Ok(());
            }

            if let Some(output) = output {
                print_output(output)?;
            } else {
                println!("No output is focused.");
            }
        }
        Msg::PickWindow => {
            let Response::PickedWindow(window) = response else {
                bail!("unexpected response: expected PickedWindow, got {response:?}");
            };

            if json {
                let window = serde_json::to_string(&window).context("error formatting response")?;
                println!("{window}");
                return Ok(());
            }

            if let Some(window) = window {
                print_window(&window);
            } else {
                println!("No window selected.");
            }
        }
        Msg::PickColor => {
            let Response::PickedColor(color) = response else {
                bail!("unexpected response: expected PickedColor, got {response:?}");
            };

            if json {
                let color = serde_json::to_string(&color).context("error formatting response")?;
                println!("{color}");
                return Ok(());
            }

            if let Some(color) = color {
                let [r, g, b] = color.rgb.map(|v| (v.clamp(0., 1.) * 255.).round() as u8);

                println!("Picked color: rgb({r}, {g}, {b})",);
                println!("Hex: #{r:02x}{g:02x}{b:02x}");
            } else {
                println!("No color was picked.");
            }
        }
        Msg::Action { .. } => {
            let Response::Handled = response else {
                bail!("unexpected response: expected Handled, got {response:?}");
            };
        }
        Msg::Output { output, .. } => {
            let Response::OutputConfigChanged(response) = response else {
                bail!("unexpected response: expected OutputConfigChanged, got {response:?}");
            };

            if json {
                let response =
                    serde_json::to_string(&response).context("error formatting response")?;
                println!("{response}");
                return Ok(());
            }

            if response == OutputConfigChanged::OutputWasMissing {
                println!("Output \"{output}\" is not connected.");
                println!("The change will apply when it is connected.");
            }
        }
        Msg::Workspaces => {
            let Response::Workspaces(mut response) = response else {
                bail!("unexpected response: expected Workspaces, got {response:?}");
            };

            if json {
                let response =
                    serde_json::to_string(&response).context("error formatting response")?;
                println!("{response}");
                return Ok(());
            }

            if response.is_empty() {
                println!("No workspaces.");
                return Ok(());
            }

            response.sort_by_key(|ws| ws.idx);
            response.sort_by(|a, b| a.output.cmp(&b.output));

            let mut current_output = if let Some(output) = response[0].output.as_deref() {
                println!("Output \"{output}\":");
                Some(output)
            } else {
                println!("No output:");
                None
            };

            for ws in &response {
                if ws.output.as_deref() != current_output {
                    let output = ws.output.as_deref().context(
                        "invalid response: workspace with no output \
                         following a workspace with an output",
                    )?;
                    current_output = Some(output);
                    println!("\nOutput \"{output}\":");
                }

                let is_active = if ws.is_active { " * " } else { "   " };
                let idx = ws.idx;
                let name = if let Some(name) = ws.name.as_deref() {
                    format!(" \"{name}\"")
                } else {
                    String::new()
                };
                println!("{is_active}{idx}{name}");
            }
        }
        Msg::KeyboardLayouts => {
            let Response::KeyboardLayouts(response) = response else {
                bail!("unexpected response: expected KeyboardLayouts, got {response:?}");
            };

            if json {
                let response =
                    serde_json::to_string(&response).context("error formatting response")?;
                println!("{response}");
                return Ok(());
            }

            let KeyboardLayouts { names, current_idx } = response;
            let current_idx = usize::from(current_idx);

            println!("Keyboard layouts:");
            for (idx, name) in names.iter().enumerate() {
                let is_active = if idx == current_idx { " * " } else { "   " };
                println!("{is_active}{idx} {name}");
            }
        }
        Msg::EventStream => {
            let Response::Handled = response else {
                bail!("unexpected response: expected Handled, got {response:?}");
            };

            if !json {
                println!("Started reading events.");
            }

            let mut read_event = socket.read_events();
            loop {
                let event = read_event().context("error reading event from niri")?;

                if json {
                    let event = serde_json::to_string(&event).context("error formatting event")?;
                    println!("{event}");
                    continue;
                }

                match event {
                    Event::WorkspacesChanged { workspaces } => {
                        println!("Workspaces changed: {workspaces:?}");
                    }
                    Event::WorkspaceUrgencyChanged { id, urgent } => {
                        println!("Workspace {id}: urgency changed to {urgent}");
                    }
                    Event::WorkspaceActivated { id, focused } => {
                        let word = if focused { "focused" } else { "activated" };
                        println!("Workspace {word}: {id}");
                    }
                    Event::WorkspaceActiveWindowChanged {
                        workspace_id,
                        active_window_id,
                    } => {
                        println!(
                            "Workspace {workspace_id}: \
                             active window changed to {active_window_id:?}"
                        );
                    }
                    Event::WindowsChanged { windows } => {
                        println!("Windows changed: {windows:?}");
                    }
                    Event::WindowOpenedOrChanged { window } => {
                        println!("Window opened or changed: {window:?}");
                    }
                    Event::WindowClosed { id } => {
                        println!("Window closed: {id}");
                    }
                    Event::WindowFocusChanged { id } => {
                        println!("Window focus changed: {id:?}");
                    }
                    Event::WindowFocusTimestampChanged {
                        id,
                        focus_timestamp,
                    } => {
                        println!("Window {id}: focus timestamp changed to {focus_timestamp:?}");
                    }
                    Event::WindowUrgencyChanged { id, urgent } => {
                        println!("Window {id}: urgency changed to {urgent}");
                    }
                    Event::WindowLayoutsChanged { changes } => {
                        println!("Window layouts changed: {changes:?}");
                    }
                    Event::KeyboardLayoutsChanged { keyboard_layouts } => {
                        println!("Keyboard layouts changed: {keyboard_layouts:?}");
                    }
                    Event::KeyboardLayoutSwitched { idx } => {
                        println!("Keyboard layout switched: {idx}");
                    }
                    Event::OverviewOpenedOrClosed { is_open: opened } => {
                        println!("Overview toggled: {opened}");
                    }
                    Event::ConfigLoaded { failed } => {
                        let status = if failed {
                            "with an error"
                        } else {
                            "successfully"
                        };
                        println!("Config loaded {status}");
                    }
                    Event::ScreenshotCaptured { path } => {
                        let mut parts = vec![];
                        parts.push("copied to clipboard".to_string());
                        if let Some(path) = &path {
                            parts.push(format!("saved to {path}"));
                        }
                        let description = parts.join(" and ");
                        println!("Screenshot captured: {description}");
                    }
                    Event::CastsChanged { casts } => {
                        println!("Casts changed: {casts:?}");
                    }
                    Event::CastStartedOrChanged { cast } => {
                        println!("Cast started or changed: {cast:?}");
                    }
                    Event::CastStopped { stream_id } => {
                        println!("Cast stopped: stream id {stream_id}");
                    }
                }
            }
        }
        Msg::OverviewState => {
            let Response::OverviewState(response) = response else {
                bail!("unexpected response: expected Overview, got {response:?}");
            };

            if json {
                let response =
                    serde_json::to_string(&response).context("error formatting response")?;
                println!("{response}");
                return Ok(());
            }

            let Overview { is_open } = response;
            if is_open {
                println!("Overview is open.");
            } else {
                println!("Overview is closed.");
            }
        }
        Msg::Casts => {
            let Response::Casts(mut casts) = response else {
                bail!("unexpected response: expected Casts, got {response:?}");
            };

            if json {
                let casts = serde_json::to_string(&casts).context("error formatting response")?;
                println!("{casts}");
                return Ok(());
            }

            if casts.is_empty() {
                println!("No screencasts.");
                return Ok(());
            }

            casts.sort_by_key(|c| (c.session_id, c.stream_id));
            for cast in casts {
                print_cast(&cast);
                println!();
            }
        }
        Msg::RemoteWindows => {
            let Response::RemoteWindows(mut windows) = response else {
                bail!("unexpected response: expected RemoteWindows, got {response:?}");
            };

            if json {
                let windows =
                    serde_json::to_string(&windows).context("error formatting response")?;
                println!("{windows}");
                return Ok(());
            }

            if windows.is_empty() {
                println!("No remote windows.");
                return Ok(());
            }

            windows.sort_by_key(|window| window.id);
            for window in windows {
                println!(
                    "Remote window {}: peer={} remote={} title={:?} app_id={:?} size={}x{} stream={}{}",
                    window.id,
                    window.peer_id,
                    window.remote_window_id,
                    window.title,
                    window.app_id,
                    window.size.0,
                    window.size.1,
                    window.stream_id,
                    if window.is_focused { " focused" } else { "" },
                );
            }
        }
        Msg::SharedWindowStreams => {
            let Response::SharedWindowStreams(mut streams) = response else {
                bail!("unexpected response: expected SharedWindowStreams, got {response:?}");
            };

            if json {
                let streams =
                    serde_json::to_string(&streams).context("error formatting response")?;
                println!("{streams}");
                return Ok(());
            }

            if streams.is_empty() {
                println!("No shared window streams.");
                return Ok(());
            }

            streams.sort_by_key(|stream| stream.window_id);
            for stream in streams {
                println!(
                    "Shared window {}: stream={}",
                    stream.window_id, stream.stream_id
                );
            }
        }
        Msg::VirtualCursors => {
            let Response::VirtualCursors(mut cursors) = response else {
                bail!("unexpected response: expected VirtualCursors, got {response:?}");
            };

            if json {
                let cursors =
                    serde_json::to_string(&cursors).context("error formatting response")?;
                println!("{cursors}");
                return Ok(());
            }

            if cursors.is_empty() {
                println!("No virtual cursors.");
                return Ok(());
            }

            cursors.sort_by(|a, b| a.cursor_id.cmp(&b.cursor_id));
            for cursor in cursors {
                println!(
                    "{}: window {} at ({:.1}, {:.1}), {:?}, size {}, visible {}",
                    cursor.cursor_id,
                    cursor.window_id,
                    cursor.x,
                    cursor.y,
                    cursor.appearance.shape,
                    cursor.appearance.size,
                    cursor.visible
                );
            }
        }
        Msg::CreateVirtualCursor { .. } => {
            let Response::VirtualCursorCreated(cursor) = response else {
                bail!("unexpected response: expected VirtualCursorCreated, got {response:?}");
            };

            if json {
                let cursor = serde_json::to_string(&cursor).context("error formatting response")?;
                println!("{cursor}");
                return Ok(());
            }

            println!("Created virtual cursor {}.", cursor.cursor_id);
        }
        Msg::SetHardwareCursor { .. } | Msg::ClearHardwareCursor => {
            let Response::Handled = response else {
                bail!("unexpected response: expected Handled, got {response:?}");
            };

            if json {
                println!("{}", json!({ "ok": true }));
            }
        }
        Msg::UpdateVirtualCursor { .. } => {
            let Response::VirtualCursorUpdated(cursor) = response else {
                bail!("unexpected response: expected VirtualCursorUpdated, got {response:?}");
            };

            if json {
                let cursor = serde_json::to_string(&cursor).context("error formatting response")?;
                println!("{cursor}");
                return Ok(());
            }

            println!("Updated virtual cursor {}.", cursor.cursor_id);
        }
        Msg::DestroyVirtualCursor { .. } => {
            let Response::VirtualCursorDestroyed { cursor_id } = response else {
                bail!("unexpected response: expected VirtualCursorDestroyed, got {response:?}");
            };

            if json {
                let response =
                    serde_json::to_string(&cursor_id).context("error formatting response")?;
                println!("{response}");
                return Ok(());
            }

            println!("Destroyed virtual cursor {cursor_id}.");
        }
        Msg::CursorOverlays => {
            let Response::CursorOverlays(mut overlays) = response else {
                bail!("unexpected response: expected CursorOverlays, got {response:?}");
            };

            if json {
                let overlays =
                    serde_json::to_string(&overlays).context("error formatting response")?;
                println!("{overlays}");
                return Ok(());
            }

            if overlays.is_empty() {
                println!("No cursor overlays.");
                return Ok(());
            }

            overlays.sort_by(|a, b| a.overlay_id.cmp(&b.overlay_id));
            for overlay in overlays {
                let anchor = match &overlay.anchor {
                    CursorOverlayAnchor::HardwarePointer => "hardware pointer".to_owned(),
                    CursorOverlayAnchor::VirtualCursor { cursor_id } => {
                        format!("virtual cursor {cursor_id}")
                    }
                };
                let resolved = overlay
                    .resolved_output
                    .as_deref()
                    .map(|output| format!(" on {output}"))
                    .unwrap_or_default();
                println!(
                    "{}: namespace \"{}\", anchored to {}, {:?}/{:?}, visible {}, interactive {}, keyboard-focus {}{}",
                    overlay.overlay_id,
                    overlay.layer_namespace,
                    anchor,
                    overlay.placement.side,
                    overlay.placement.align,
                    overlay.visible,
                    overlay.interactive,
                    overlay.keyboard_focus,
                    resolved
                );
            }
        }
        Msg::RegisterCursorOverlay { .. } => {
            let Response::CursorOverlayRegistered(overlay) = response else {
                bail!("unexpected response: expected CursorOverlayRegistered, got {response:?}");
            };

            if json {
                let overlay =
                    serde_json::to_string(&overlay).context("error formatting response")?;
                println!("{overlay}");
                return Ok(());
            }

            println!("Registered cursor overlay {}.", overlay.overlay_id);
        }
        Msg::UpdateCursorOverlay { .. } => {
            let Response::CursorOverlayUpdated(overlay) = response else {
                bail!("unexpected response: expected CursorOverlayUpdated, got {response:?}");
            };

            if json {
                let overlay =
                    serde_json::to_string(&overlay).context("error formatting response")?;
                println!("{overlay}");
                return Ok(());
            }

            println!("Updated cursor overlay {}.", overlay.overlay_id);
        }
        Msg::UnregisterCursorOverlay { .. } => {
            let Response::CursorOverlayUnregistered { overlay_id } = response else {
                bail!("unexpected response: expected CursorOverlayUnregistered, got {response:?}");
            };

            if json {
                let response =
                    serde_json::to_string(&overlay_id).context("error formatting response")?;
                println!("{response}");
                return Ok(());
            }

            println!("Unregistered cursor overlay {overlay_id}.");
        }
    }

    Ok(())
}

fn cursor_overlay_anchor(
    hardware_pointer: bool,
    virtual_cursor: &Option<String>,
) -> anyhow::Result<CursorOverlayAnchor> {
    cursor_overlay_anchor_opt(hardware_pointer, virtual_cursor)?
        .ok_or_else(|| anyhow!("must specify --anchor-hardware-pointer or --anchor-virtual-cursor"))
}

fn cursor_overlay_anchor_opt(
    hardware_pointer: bool,
    virtual_cursor: &Option<String>,
) -> anyhow::Result<Option<CursorOverlayAnchor>> {
    match (hardware_pointer, virtual_cursor) {
        (true, None) => Ok(Some(CursorOverlayAnchor::HardwarePointer)),
        (false, Some(cursor_id)) => Ok(Some(CursorOverlayAnchor::VirtualCursor {
            cursor_id: cursor_id.clone(),
        })),
        (false, None) => Ok(None),
        (true, Some(_)) => bail!("cannot specify both hardware and virtual cursor anchors"),
    }
}

#[allow(clippy::too_many_arguments)]
fn cursor_overlay_placement_opt(
    side: Option<niri_ipc::CursorOverlaySide>,
    align: Option<niri_ipc::CursorOverlayAlign>,
    gap: Option<f64>,
    offset_x: Option<f64>,
    offset_y: Option<f64>,
    edge_padding: Option<f64>,
    flip: bool,
    no_flip: bool,
) -> Option<CursorOverlayPlacement> {
    if side.is_none()
        && align.is_none()
        && gap.is_none()
        && offset_x.is_none()
        && offset_y.is_none()
        && edge_padding.is_none()
        && !flip
        && !no_flip
    {
        return None;
    }

    let default = CursorOverlayPlacement::default();
    Some(CursorOverlayPlacement {
        side: side.unwrap_or(default.side),
        align: align.unwrap_or(default.align),
        gap: gap.unwrap_or(default.gap),
        offset_x: offset_x.unwrap_or(default.offset_x),
        offset_y: offset_y.unwrap_or(default.offset_y),
        edge_padding: edge_padding.unwrap_or(default.edge_padding),
        flip: if no_flip { false } else { flip || default.flip },
    })
}

fn print_output(output: Output) -> anyhow::Result<()> {
    let Output {
        name,
        make,
        model,
        serial,
        physical_size,
        modes,
        current_mode,
        is_custom_mode,
        vrr_supported,
        vrr_enabled,
        logical,
    } = output;

    let serial = serial.as_deref().unwrap_or("Unknown");
    println!(r#"Output "{make} {model} {serial}" ({name})"#);

    let print_qualifier = |is_preferred: bool, is_current: bool, is_custom_mode: bool| {
        let mut qualifier = Vec::new();
        if is_current {
            qualifier.push("current");
            if is_custom_mode {
                qualifier.push("custom");
            };
        };

        if is_preferred {
            qualifier.push("preferred");
        };

        if qualifier.is_empty() {
            String::new()
        } else {
            format!(" ({})", qualifier.join(", "))
        }
    };

    if let Some(current) = current_mode {
        let mode = *modes
            .get(current)
            .context("invalid response: current mode does not exist")?;
        let Mode {
            width,
            height,
            refresh_rate,
            is_preferred,
        } = mode;
        let refresh = refresh_rate as f64 / 1000.;

        // This is technically the current mode, but the println below already specifies that.
        let qualifier = print_qualifier(is_preferred, false, is_custom_mode);
        println!("  Current mode: {width}x{height} @ {refresh:.3} Hz{qualifier}");
    } else {
        println!("  Disabled");
    }

    if vrr_supported {
        let enabled = if vrr_enabled { "enabled" } else { "disabled" };
        println!("  Variable refresh rate: supported, {enabled}");
    } else {
        println!("  Variable refresh rate: not supported");
    }

    if let Some((width, height)) = physical_size {
        println!("  Physical size: {width}x{height} mm");
    } else {
        println!("  Physical size: unknown");
    }

    if let Some(logical) = logical {
        let LogicalOutput {
            x,
            y,
            width,
            height,
            scale,
            transform,
        } = logical;
        println!("  Logical position: {x}, {y}");
        println!("  Logical size: {width}x{height}");
        println!("  Scale: {scale}");

        let transform = match transform {
            Transform::Normal => "normal",
            Transform::_90 => "90° counter-clockwise",
            Transform::_180 => "180°",
            Transform::_270 => "270° counter-clockwise",
            Transform::Flipped => "flipped horizontally",
            Transform::Flipped90 => "90° counter-clockwise, flipped horizontally",
            Transform::Flipped180 => "flipped vertically",
            Transform::Flipped270 => "270° counter-clockwise, flipped horizontally",
        };
        println!("  Transform: {transform}");
    }

    println!("  Available modes:");
    for (idx, mode) in modes.into_iter().enumerate() {
        let Mode {
            width,
            height,
            refresh_rate,
            is_preferred,
        } = mode;
        let refresh = refresh_rate as f64 / 1000.;

        let is_current = Some(idx) == current_mode;
        let qualifier = print_qualifier(is_preferred, is_current, is_custom_mode);

        println!("    {width}x{height}@{refresh:.3}{qualifier}");
    }
    Ok(())
}

fn print_window(window: &Window) {
    let focused = if window.is_focused { " (focused)" } else { "" };
    let urgent = if window.is_urgent { " (urgent)" } else { "" };
    println!("Window ID {}:{focused}{urgent}", window.id);

    if let Some(title) = &window.title {
        println!("  Title: \"{title}\"");
    } else {
        println!("  Title: (unset)");
    }

    if let Some(app_id) = &window.app_id {
        println!("  App ID: \"{app_id}\"");
    } else {
        println!("  App ID: (unset)");
    }

    println!(
        "  Is floating: {}",
        if window.is_floating { "yes" } else { "no" }
    );

    if let Some(pid) = window.pid {
        println!("  PID: {pid}");
    } else {
        println!("  PID: (unknown)");
    }

    if let Some(workspace_id) = window.workspace_id {
        println!("  Workspace ID: {workspace_id}");
    } else {
        println!("  Workspace ID: (none)");
    }

    let WindowLayout {
        pos_in_scrolling_layout,
        tile_size,
        window_size,
        tile_pos_in_workspace_view,
        window_offset_in_tile,
    } = window.layout;

    println!("  Layout:");
    println!(
        "    Tile size: {} x {}",
        fmt_rounded(tile_size.0),
        fmt_rounded(tile_size.1)
    );

    if let Some(pos) = pos_in_scrolling_layout {
        println!("    Scrolling position: column {}, tile {}", pos.0, pos.1);
    }

    if let Some(pos) = tile_pos_in_workspace_view {
        println!(
            "    Workspace-view position: {}, {}",
            fmt_rounded(pos.0),
            fmt_rounded(pos.1)
        );
    }

    println!("    Window size: {} x {}", window_size.0, window_size.1);
    println!(
        "    Window offset in tile: {} x {}",
        fmt_rounded(window_offset_in_tile.0),
        fmt_rounded(window_offset_in_tile.1)
    );
}

fn print_cast(cast: &Cast) {
    let active = if cast.is_active { "" } else { " (inactive)" };
    println!("Cast stream ID {}:{active}", cast.stream_id);
    println!("  Session ID: {}", cast.session_id);

    let kind = match cast.kind {
        CastKind::PipeWire => "PipeWire",
        CastKind::WlrScreencopy => "wlr-screencopy",
    };
    println!("  Kind: {kind}");

    match &cast.target {
        CastTarget::Nothing {} => {
            println!("  Target: nothing (cleared)");
        }
        CastTarget::Output { name } => {
            println!("  Target: output \"{name}\"");
        }
        CastTarget::Window { id } => {
            println!("  Target: window {id}");
        }
    }

    if cast.is_dynamic_target {
        println!("  Dynamic cast target");
    }

    if let Some(pid) = cast.pid {
        println!("  PID: {pid}");
    }

    if let Some(node_id) = cast.pw_node_id {
        println!("  PipeWire node ID: {node_id}");
    }
}

fn fmt_rounded(x: f64) -> String {
    let r = x.round();
    if (r - x).abs() <= 0.005 {
        format!("{r}")
    } else {
        format!("{x:.2}")
    }
}

fn ensure_absolute_path(path: &mut String) -> anyhow::Result<()> {
    let p = Path::new(path);
    if p.is_relative() {
        let mut cwd = env::current_dir().context("error getting current working directory")?;
        cwd.push(p);
        match cwd.into_os_string().into_string() {
            Ok(absolute) => *path = absolute,
            Err(cwd) => bail!("couldn't convert absolute path to string: {cwd:?}"),
        }
    }
    Ok(())
}

fn virtual_cursor_source(
    shape: Option<niri_ipc::VirtualCursorShape>,
    cursor_theme: Option<String>,
    cursor_icon: Option<String>,
) -> VirtualCursorSource {
    if let Some(shape) = shape {
        return VirtualCursorSource::Builtin { shape };
    }

    let icon = cursor_icon.and_then(|icon| {
        let icon = icon.trim().to_owned();
        (!icon.is_empty()).then_some(icon)
    });
    let theme = cursor_theme.and_then(|theme| {
        let theme = theme.trim().to_owned();
        (!theme.is_empty()).then_some(theme)
    });
    VirtualCursorSource::Theme { theme, icon }
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn parse_rgba_color_opt(color: Option<&str>) -> anyhow::Result<Option<RgbaColor>> {
    color.map(parse_rgba_color).transpose()
}

fn parse_rgba_color(color: &str) -> anyhow::Result<RgbaColor> {
    let hex = color
        .strip_prefix('#')
        .ok_or_else(|| anyhow!("color must start with #"))?;
    let expanded;
    let hex = match hex.len() {
        3 => {
            expanded = hex
                .chars()
                .flat_map(|c| [c, c])
                .chain(['f', 'f'])
                .collect::<String>();
            expanded.as_str()
        }
        6 => {
            expanded = format!("{hex}ff");
            expanded.as_str()
        }
        8 => hex,
        _ => bail!("color must be #rgb, #rrggbb, or #rrggbbaa"),
    };

    let value = u32::from_str_radix(hex, 16).context("error parsing color")?;
    let r = ((value >> 24) & 0xff) as f32 / 255.;
    let g = ((value >> 16) & 0xff) as f32 / 255.;
    let b = ((value >> 8) & 0xff) as f32 / 255.;
    let a = (value & 0xff) as f32 / 255.;
    Ok(RgbaColor { r, g, b, a })
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_fmt_rounded() {
        assert_snapshot!(fmt_rounded(1.9), @"1.90");
        assert_snapshot!(fmt_rounded(1.994), @"1.99");
        assert_snapshot!(fmt_rounded(1.996), @"2");
        assert_snapshot!(fmt_rounded(2.0), @"2");
        assert_snapshot!(fmt_rounded(2.004), @"2");
        assert_snapshot!(fmt_rounded(2.006), @"2.01");
        assert_snapshot!(fmt_rounded(2.1), @"2.10");
    }
}
