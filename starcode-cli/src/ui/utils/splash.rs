use crossterm::{
    cursor, event, execute,
    style::{Color as CtColor, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use ratatui::{
    layout::{HorizontalAlignment as Alignment, Rect},
    prelude::CrosstermBackend,
    style::{Color as TuiColor, Style},
    text::{Line, Text},
    widgets::Paragraph,
    Terminal as RatatuiTerminal,
};
use std::future::Future;
use std::io::{stdout, Write};
use std::time::Instant;
use tokio::time::{sleep, Duration};

pub const LOGO: &[&str] = &[
    "  █▀▀ █▀█ █▀█ █▀▄▀█ █▀▄▀█ ▀█▀ █▀█ █▀▄",
    "  █▄▄ █▄█ █▀▄ █░▀░█ █░▀░█ ░█░ █▄█ █▄▀",
];

const TIPS: &[&str] = &[
    "/help for help  ·  Esc to interrupt  ·  --resume <id> to resume",
];

fn splash_disabled() -> bool {
    std::env::var("STAR_NO_SPLASH")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn splash_ms(default: u64) -> u64 {
    std::env::var("STAR_SPLASH_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn dots_str(tick: usize) -> &'static str {
    match tick % 4 {
        0 => "   ",
        1 => ".  ",
        2 => ".. ",
        _ => "...",
    }
}

fn status_text(tick: usize) -> String {
    format!("Starting{}  (press any key to skip)", dots_str(tick))
}

fn cleanup_terminal(out: &mut std::io::Stdout) {
    let _ = execute!(
        out,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Show
    );
    let _ = terminal::disable_raw_mode();
}

fn draw_static(out: &mut std::io::Stdout) -> Result<(u16, u16), Box<dyn std::error::Error>> {
    let version = env!("CARGO_PKG_VERSION");

    let (tw, th) = terminal::size().unwrap_or((100, 30));

    let mut lines: Vec<String> = Vec::new();
    for l in LOGO {
        lines.push(l.to_string());
    }
    lines.push("".to_string());
    lines.push("".to_string());
    lines.push(format!("  v{} · AI Coding Agent", version));
    lines.push("".to_string());
    for t in TIPS {
        lines.push(t.to_string());
    }

    let max_len = lines.iter().map(|s| s.len()).max().unwrap_or(0) as u16;
    let start_y = (th.saturating_sub(lines.len() as u16 + 2)).saturating_div(2);
    let start_x = tw.saturating_sub(max_len).saturating_div(2);

    execute!(out, cursor::MoveTo(0, 0), terminal::Clear(ClearType::All))?;
    for (idx, s) in lines.iter().enumerate() {
        let y = start_y + idx as u16;
        let x = if s.len() as u16 >= tw { 0 } else { start_x };
        let color = if idx < LOGO.len() {
            CtColor::Cyan
        } else if s.contains("v") && s.contains("AI Coding") {
            CtColor::DarkYellow
        } else {
            CtColor::DarkGrey
        };
        execute!(
            out,
            cursor::MoveTo(x, y),
            SetForegroundColor(color),
            Print(s),
            ResetColor
        )?;
    }
    out.flush()?;

    Ok((tw, th))
}

pub async fn run_splash() -> Result<(), Box<dyn std::error::Error>> {
    run_splash_until(async {
        sleep(Duration::from_millis(splash_ms(1200))).await;
        Ok(())
    })
    .await
}

pub async fn run_splash_in_alt(
    out: &mut std::io::Stdout,
) -> Result<(), Box<dyn std::error::Error>> {
    if splash_disabled() {
        return Ok(());
    }

    execute!(out, cursor::Hide)?;

    let (_, th) = draw_static(out)?;

    let total_ms: u64 = splash_ms(1200);
    let ticks = (total_ms / 55).max(1);

    for i in 0..ticks {
        if event::poll(std::time::Duration::from_millis(0))? {
            let ev = event::read()?;
            if matches!(ev, event::Event::Key(_)) {
                break;
            }
        }

        execute!(
            out,
            cursor::MoveTo(0, th.saturating_sub(2)),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(CtColor::DarkGrey),
            Print(status_text(i as usize)),
            ResetColor
        )?;
        out.flush()?;
        sleep(Duration::from_millis(55)).await;
    }

    execute!(out, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    Ok(())
}

pub async fn run_splash_ratatui(
    terminal: &mut RatatuiTerminal<CrosstermBackend<std::io::Stdout>>,
) -> Result<(), Box<dyn std::error::Error>> {
    if splash_disabled() {
        return Ok(());
    }

    let version = env!("CARGO_PKG_VERSION");

    let total_ms: u64 = splash_ms(1500);
    let skip_delay_ms: u64 = std::env::var("STAR_SPLASH_SKIP_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300u64);
    let ticks = (total_ms / 55).max(1);

    // Clear any residual key events
    while event::poll(std::time::Duration::from_millis(0))? {
        let _ = event::read();
    }

    let started = Instant::now();

    for i in 0..ticks {
        terminal.draw(|f| {
            let size = f.area();

            // Compact layout: logo + blank + version + blank + hint
            let logo_height = LOGO.len() as u16 + 4; // logo + blank + version + blank + hint
            let vertical_margin = size.height.saturating_sub(logo_height) / 2;
            let popup_area = Rect {
                x: size.x,
                y: size.y + vertical_margin,
                width: size.width,
                height: logo_height,
            };

            let mut text = Text::default();
            // Logo
            for l in LOGO {
                text.lines
                    .push(Line::from(*l).style(Style::default().fg(TuiColor::Cyan)));
            }
            text.lines.push(Line::from(""));
            // Version
            text.lines.push(
                Line::from(format!("  v{} · AI Coding Agent", version))
                    .style(Style::default().fg(TuiColor::Yellow)),
            );
            text.lines.push(Line::from(""));
            // Hint
            for t in TIPS {
                text.lines
                    .push(Line::from(*t).style(Style::default().fg(TuiColor::DarkGray)));
            }

            let para = Paragraph::new(text)
                .alignment(Alignment::Center);
            f.render_widget(para, popup_area);

            // Status line at bottom
            let status = Paragraph::new(status_text(i as usize))
                .alignment(Alignment::Center)
                .style(Style::default().fg(TuiColor::DarkGray));
            f.render_widget(status, Rect {
                x: size.x,
                y: size.y + size.height.saturating_sub(1),
                width: size.width,
                height: 1,
            });
        })?;

        // After first frame, allow skip; skip_delay_ms prevents accidental skip
        if started.elapsed().as_millis() as u64 >= skip_delay_ms {
            if event::poll(std::time::Duration::from_millis(0))? {
                let ev = event::read()?;
                if matches!(ev, event::Event::Key(_)) {
                    break;
                }
            }
        }

        sleep(Duration::from_millis(55)).await;
    }

    Ok(())
}

pub async fn run_splash_until<T, Fut>(fut: Fut) -> Result<T, Box<dyn std::error::Error>>
where
    Fut: Future<Output = Result<T, Box<dyn std::error::Error>>> + Send,
    T: Send,
{
    if splash_disabled() {
        return fut.await;
    }

    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(
        out,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0),
        cursor::Hide
    )?;

    let (tw, th) = match draw_static(&mut out) {
        Ok(v) => v,
        Err(e) => {
            cleanup_terminal(&mut out);
            return Err(e);
        }
    };

    let min_ms: u64 = std::env::var("STAR_SPLASH_MIN_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(800u64);
    let started = Instant::now();
    let _ = tw;

    let mut pinned = Box::pin(fut);
    let mut i: usize = 0;

    let print_status = |i: usize, th: u16, out: &mut std::io::Stdout| -> std::io::Result<()> {
        execute!(
            out,
            cursor::MoveTo(0, th.saturating_sub(2)),
            terminal::Clear(ClearType::CurrentLine),
            SetForegroundColor(CtColor::DarkGrey),
            Print(status_text(i)),
            ResetColor
        )?;
        out.flush()
    };

    loop {
        tokio::select! {
            res = &mut pinned => {
                let elapsed = started.elapsed().as_millis() as u64;
                if elapsed < min_ms {
                    let remaining = min_ms.saturating_sub(elapsed);
                    let ticks = (remaining / 55).max(1);
                    for _ in 0..ticks {
                        if event::poll(std::time::Duration::from_millis(0))? {
                            let ev = event::read()?;
                            if matches!(ev, event::Event::Key(_)) {
                                break;
                            }
                        }
                        let _ = print_status(i, th, &mut out);
                        i = i.wrapping_add(1);
                        sleep(Duration::from_millis(55)).await;
                    }
                }

                cleanup_terminal(&mut out);
                return res;
            }
            _ = sleep(Duration::from_millis(55)) => {
                if event::poll(std::time::Duration::from_millis(0))? {
                    let ev = event::read()?;
                    if matches!(ev, event::Event::Key(_)) {
                        break;
                    }
                }
                let _ = print_status(i, th, &mut out);
                i = i.wrapping_add(1);
            }
        }
    }

    cleanup_terminal(&mut out);
    pinned.await
}
