use serialport::{ClearBuffer, SerialPort};
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const ROWS: usize = 32;
const COLS: usize = 32;
const CELLS: usize = ROWS * COLS;
const CELL_DEAD: u32 = 0x0000_0000;
const CELL_ALIVE: u32 = 0x00FF_FFFF;

const FRAME_HEADER_0: u8 = 0xAB;
const FRAME_HEADER_1: u8 = 0xCD;
const FRAME_PAYLOAD_BYTES: usize = CELLS * 4;

const HOST_CMD_SEND_GRID: u8 = 0x01;
const HOST_CMD_STEP_ONCE: u8 = 0x02;

#[derive(Clone)]
struct PatternCase {
    name: &'static str,
    coords: Vec<(usize, usize)>,
    steps: usize,
    toroidal_check: bool,
}

#[derive(Clone, Copy)]
struct EdgeAliveCounts {
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
}

fn idx(row: usize, col: usize) -> usize {
    row * COLS + col
}

fn build_grid_from_coords(coords: &[(usize, usize)]) -> Vec<u32> {
    let mut grid = vec![CELL_DEAD; CELLS];
    for &(r, c) in coords {
        grid[idx(r, c)] = CELL_ALIVE;
    }
    grid
}

fn oracle_step(cur: &[u32]) -> Vec<u32> {
    let mut nxt = vec![CELL_DEAD; CELLS];

    for row in 0..ROWS {
        for col in 0..COLS {
            let mut neighbors = 0u32;

            for dr in [-1isize, 0, 1] {
                for dc in [-1isize, 0, 1] {
                    if dr == 0 && dc == 0 {
                        continue;
                    }

                    let nr = ((row as isize + dr + ROWS as isize) as usize) % ROWS;
                    let nc = ((col as isize + dc + COLS as isize) as usize) % COLS;
                    if cur[idx(nr, nc)] == CELL_ALIVE {
                        neighbors += 1;
                    }
                }
            }

            let alive = cur[idx(row, col)] == CELL_ALIVE;
            nxt[idx(row, col)] = if alive {
                if neighbors == 2 || neighbors == 3 {
                    CELL_ALIVE
                } else {
                    CELL_DEAD
                }
            } else if neighbors == 3 {
                CELL_ALIVE
            } else {
                CELL_DEAD
            };
        }
    }

    nxt
}

fn render_ascii_plain(grid: &[u32]) -> String {
    let mut out = String::new();
    for row in 0..ROWS {
        for col in 0..COLS {
            if grid[idx(row, col)] == CELL_ALIVE {
                out.push_str("##");
            } else {
                out.push_str("..");
            }
        }
        out.push('\n');
    }
    out
}

fn first_diff(actual: &[u32], expected: &[u32]) -> Option<(usize, usize, u32, u32)> {
    actual
        .iter()
        .zip(expected.iter())
        .enumerate()
        .find_map(|(i, (a, e))| {
            if a != e {
                Some((i / COLS, i % COLS, *a, *e))
            } else {
                None
            }
        })
}

fn edge_alive_counts(grid: &[u32]) -> EdgeAliveCounts {
    let top = (0..COLS)
        .filter(|&col| grid[idx(0, col)] == CELL_ALIVE)
        .count();
    let bottom = (0..COLS)
        .filter(|&col| grid[idx(ROWS - 1, col)] == CELL_ALIVE)
        .count();
    let left = (0..ROWS)
        .filter(|&row| grid[idx(row, 0)] == CELL_ALIVE)
        .count();
    let right = (0..ROWS)
        .filter(|&row| grid[idx(row, COLS - 1)] == CELL_ALIVE)
        .count();

    EdgeAliveCounts {
        top,
        bottom,
        left,
        right,
    }
}

fn has_top_or_left_activity(counts: EdgeAliveCounts) -> bool {
    counts.top > 0 || counts.left > 0
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let golrs_dir = manifest
        .parent()
        .ok_or_else(|| "failed to resolve GoLRS directory".to_string())?;
    let root = golrs_dir
        .parent()
        .ok_or_else(|| "failed to resolve workspace root".to_string())?;
    Ok(root.to_path_buf())
}

fn open_named_log(file_name: &str) -> Result<(BufWriter<File>, PathBuf), String> {
    let mut path = workspace_root()?;
    path.push(file_name);

    let file = File::create(&path)
        .map_err(|e| format!("failed to create log file {}: {e}", path.display()))?;

    Ok((BufWriter::new(file), path))
}

fn file_line<W: Write + ?Sized>(log: &mut W, line: &str) -> Result<(), String> {
    writeln!(log, "{}", line).map_err(|e| format!("failed to write log line: {e}"))
}

fn log_line<W: Write + ?Sized>(log: &mut W, line: &str) -> Result<(), String> {
    println!("{}", line);
    file_line(log, line)
}

fn log_grid<W: Write + ?Sized>(log: &mut W, title: &str, grid: &[u32]) -> Result<(), String> {
    println!("{}", title);
    let rendered = render_ascii_plain(grid);
    print!("{}", rendered);
    file_line(log, title)?;
    write!(log, "{}", rendered).map_err(|e| format!("failed to write log grid: {e}"))
}

fn send_grid(port: &mut dyn SerialPort, grid: &[u32]) -> Result<(), String> {
    let mut data = Vec::with_capacity(2 + FRAME_PAYLOAD_BYTES);
    data.push(HOST_CMD_SEND_GRID);
    data.push(0u8);
    for &cell in grid {
        data.extend_from_slice(&cell.to_be_bytes());
    }

    port.write_all(&data)
        .map_err(|e| format!("failed to send grid command: {e}"))
}

fn send_step_once(port: &mut dyn SerialPort) -> Result<(), String> {
    port.write_all(&[HOST_CMD_STEP_ONCE])
        .map_err(|e| format!("failed to send step command: {e}"))
}

fn read_one_grid_frame(port: &mut dyn SerialPort, timeout: Duration) -> Result<Vec<u32>, String> {
    let deadline = Instant::now() + timeout;
    let mut stream = Vec::<u8>::new();
    let mut tmp = [0u8; 1024];

    loop {
        match port.read(&mut tmp) {
            Ok(n) if n > 0 => {
                stream.extend_from_slice(&tmp[..n]);

                if let Some(pos) = stream
                    .windows(2)
                    .position(|w| w[0] == FRAME_HEADER_0 && w[1] == FRAME_HEADER_1)
                {
                    let start = pos + 2;
                    let end = start + FRAME_PAYLOAD_BYTES;
                    if stream.len() >= end {
                        let payload = &stream[start..end];
                        let mut out = vec![CELL_DEAD; CELLS];
                        for (i, cell) in out.iter_mut().enumerate() {
                            let o = i * 4;
                            *cell = u32::from_be_bytes([
                                payload[o],
                                payload[o + 1],
                                payload[o + 2],
                                payload[o + 3],
                            ]);
                        }
                        return Ok(out);
                    }
                }
            }
            Ok(_) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("serial read error: {e}")),
        }

        if Instant::now() >= deadline {
            let header_pos = stream
                .windows(2)
                .position(|w| w[0] == FRAME_HEADER_0 && w[1] == FRAME_HEADER_1);

            return Err(match header_pos {
                Some(pos) => {
                    let payload_bytes = stream.len().saturating_sub(pos + 2);
                    format!(
                        "timed out while waiting for a complete grid frame: received header after {} leading byte(s), payload bytes received={}/{}",
                        pos,
                        payload_bytes.min(FRAME_PAYLOAD_BYTES),
                        FRAME_PAYLOAD_BYTES
                    )
                }
                None if stream.is_empty() => {
                    "timed out while waiting for grid frame: no serial bytes received".to_string()
                }
                None => format!(
                    "timed out while waiting for grid frame: received {} byte(s), but no frame header was found",
                    stream.len()
                ),
            });
        }
    }
}

fn try_flush_input(port: &mut dyn SerialPort) {
    let _ = port.clear(ClearBuffer::Input);
}

fn await_matching_frame(
    port: &mut dyn SerialPort,
    log: &mut dyn Write,
    context: &str,
    expected: &[u32],
    timeout: Duration,
) -> Result<Vec<u32>, String> {
    let deadline = Instant::now() + timeout;
    let mut last_seen = vec![CELL_DEAD; CELLS];
    let mut frame_count = 0usize;
    let mut last_diff = None;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let frame = match read_one_grid_frame(port, remaining) {
            Ok(frame) => frame,
            Err(err) if frame_count > 0 => {
                if let Some((r, c, got, want)) = last_diff {
                    return Err(format!(
                        "timed out waiting for a matching frame after {} mismatched frame(s); last mismatch at row={}, col={}: got=0x{got:08X}, expected=0x{want:08X}; {}\nlast received frame was:\n{}",
                        frame_count,
                        r,
                        c,
                        err,
                        render_ascii_plain(&last_seen)
                    ));
                }

                return Err(format!(
                    "timed out waiting for a matching frame after {} received frame(s); {}\nlast received frame was:\n{}",
                    frame_count,
                    err,
                    render_ascii_plain(&last_seen)
                ));
            }
            Err(err) => return Err(err),
        };

        frame_count += 1;
        log_grid(
            log,
            &format!("{} received frame #{}:", context, frame_count),
            &frame,
        )?;

        if first_diff(&frame, expected).is_none() {
            return Ok(frame);
        }
        last_diff = first_diff(&frame, expected);
        last_seen = frame;
    }

    if frame_count == 0 {
        Err("timed out waiting for matching frame: no frames were received".to_string())
    } else if let Some((r, c, got, want)) = last_diff {
        Err(format!(
            "timed out waiting for a matching frame after {} mismatched frame(s); last mismatch at row={}, col={}: got=0x{got:08X}, expected=0x{want:08X}\nlast received frame was:\n{}",
            frame_count,
            r,
            c,
            render_ascii_plain(&last_seen)
        ))
    } else {
        Err(format!(
            "timed out waiting for a matching frame after {} received frame(s)\nlast received frame was:\n{}",
            frame_count,
            render_ascii_plain(&last_seen)
        ))
    }
}

fn build_patterns() -> Vec<PatternCase> {
    let center_r = ROWS / 2;
    let center_c = COLS / 2;

    let block = vec![
        (center_r, center_c),
        (center_r, center_c + 1),
        (center_r + 1, center_c),
        (center_r + 1, center_c + 1),
    ];

    let beehive = vec![
        (center_r, center_c + 1),
        (center_r, center_c + 2),
        (center_r + 1, center_c),
        (center_r + 1, center_c + 3),
        (center_r + 2, center_c + 1),
        (center_r + 2, center_c + 2),
    ];

    let blinker = vec![
        (center_r, center_c),
        (center_r, center_c + 1),
        (center_r, center_c + 2),
    ];

    let glider_origin_r = ROWS - 3;
    let glider_origin_c = COLS - 3;
    let glider = vec![
        (glider_origin_r, glider_origin_c + 1),
        (glider_origin_r + 1, glider_origin_c + 2),
        (glider_origin_r + 2, glider_origin_c),
        (glider_origin_r + 2, glider_origin_c + 1),
        (glider_origin_r + 2, glider_origin_c + 2),
    ];

    let pulsar_origin_r = center_r - 6;
    let pulsar_origin_c = center_c - 6;
    let pulsar = vec![
        (0, 2),
        (0, 3),
        (0, 4),
        (0, 8),
        (0, 9),
        (0, 10),
        (2, 0),
        (2, 5),
        (2, 7),
        (2, 12),
        (3, 0),
        (3, 5),
        (3, 7),
        (3, 12),
        (4, 0),
        (4, 5),
        (4, 7),
        (4, 12),
        (5, 2),
        (5, 3),
        (5, 4),
        (5, 8),
        (5, 9),
        (5, 10),
        (7, 2),
        (7, 3),
        (7, 4),
        (7, 8),
        (7, 9),
        (7, 10),
        (8, 0),
        (8, 5),
        (8, 7),
        (8, 12),
        (9, 0),
        (9, 5),
        (9, 7),
        (9, 12),
        (10, 0),
        (10, 5),
        (10, 7),
        (10, 12),
        (12, 2),
        (12, 3),
        (12, 4),
        (12, 8),
        (12, 9),
        (12, 10),
    ]
    .into_iter()
    .map(|(r, c)| (pulsar_origin_r + r, pulsar_origin_c + c))
    .collect::<Vec<_>>();

    vec![
        PatternCase {
            name: "block",
            coords: block,
            steps: 4,
            toroidal_check: false,
        },
        PatternCase {
            name: "beehive",
            coords: beehive,
            steps: 4,
            toroidal_check: false,
        },
        PatternCase {
            name: "blinker",
            coords: blinker,
            steps: 6,
            toroidal_check: false,
        },
        PatternCase {
            name: "pulsar",
            coords: pulsar,
            steps: 6,
            toroidal_check: false,
        },
        PatternCase {
            name: "glider",
            coords: glider,
            steps: 20,
            toroidal_check: true,
        },
    ]
}

fn parse_arg(args: &[String], flag: &str, default: &str) -> String {
    args.windows(2)
        .find_map(|w| {
            if w[0] == flag {
                Some(w[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| default.to_string())
}

fn normalize_value(flag: &str, value: String) -> String {
    let key = match flag {
        "--port" => "port=",
        "--baud" => "baud=",
        _ => "",
    };

    if !key.is_empty()
        && let Some(rest) = value.strip_prefix(key)
    {
        return rest.to_string();
    }

    value
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let port_name = normalize_value("--port", parse_arg(&args, "--port", "COM4"));
    let baud = normalize_value("--baud", parse_arg(&args, "--baud", "115200"))
        .parse::<u32>()
        .map_err(|e| format!("invalid baud value: {e}"))?;

    let (mut summary_log, summary_log_path) = open_named_log("test_summary.log")?;
    let mut summary_lines = Vec::<String>::new();
    let mut summary_results = Vec::<(String, bool)>::new();

    let mut port = serialport::new(&port_name, baud)
        .timeout(Duration::from_millis(40))
        .open()
        .map_err(|e| format!("failed to open serial port {port_name}: {e}"))?;

    let (mut init_log, init_log_path) = open_named_log("test_grid_init.log")?;
    log_line(
        &mut init_log,
        &format!(
            "HIL tester connected: port={}, baud={} (grid-init log: {})",
            port_name,
            baud,
            init_log_path.display()
        ),
    )?;
    summary_lines.push(format!(
        "HIL summary started: port={}, baud={} (summary log: {})",
        port_name,
        baud,
        summary_log_path.display()
    ));
    summary_lines.push(format!("grid_init log: {}", init_log_path.display()));

    let mut failures = Vec::<String>::new();
    let mut passes = Vec::<String>::new();

    try_flush_input(&mut *port);

    // Passive boot capture: read the first board-emitted frame without sending any host command.
    log_line(
        &mut init_log,
        "Capturing fresh boot output frame (no host grid injection)...",
    )?;
    match read_one_grid_frame(&mut *port, Duration::from_secs(5)) {
        Ok(boot_frame) => {
            log_grid(&mut init_log, "fresh boot output frame #1:", &boot_frame)?;
            summary_results.push(("grid_init".to_string(), true));
            summary_lines.push("grid_init PASS".to_string());
        }
        Err(err) => {
            let failure = format!("grid_init FAIL: {err}");
            log_line(&mut init_log, &failure)?;
            summary_results.push(("grid_init".to_string(), false));
            summary_lines.push(failure.clone());
            failures.push(failure);
        }
    }

    let patterns = build_patterns();

    for case in patterns {
        let safe_name: String = case
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let pattern_file = format!("test_pattern_{}.log", safe_name);
        let (mut pattern_log, pattern_log_path) = open_named_log(&pattern_file)?;

        log_line(
            &mut pattern_log,
            &format!(
                "pattern={} started (log: {})",
                case.name,
                pattern_log_path.display()
            ),
        )?;

        let pattern_result: Result<(), String> = (|| {
            let mut expected = build_grid_from_coords(&case.coords);
            let mut first_expected_wrap_step: Option<usize> = None;
            let mut first_actual_wrap_step: Option<usize> = None;

            log_grid(
                &mut pattern_log,
                &format!("pattern={} expected initial:", case.name),
                &expected,
            )?;

            if case.toroidal_check {
                log_line(
                    &mut pattern_log,
                    "toroidal-check=enabled: glider is seeded against bottom/right edges; expected wrap activity on top and/or left edges.",
                )?;
                let initial_edges = edge_alive_counts(&expected);
                log_line(
                    &mut pattern_log,
                    &format!(
                        "toroidal edge counts at step=0: top={}, bottom={}, left={}, right={}",
                        initial_edges.top,
                        initial_edges.bottom,
                        initial_edges.left,
                        initial_edges.right
                    ),
                )?;
            }

            try_flush_input(&mut *port);
            send_grid(&mut *port, &expected)?;
            let echo = await_matching_frame(
                &mut *port,
                &mut pattern_log,
                &format!("pattern={} echo", case.name),
                &expected,
                Duration::from_secs(3),
            )?;
            if let Some((r, c, got, want)) = first_diff(&echo, &expected) {
                return Err(format!(
                    "echo mismatch for pattern={} at row={}, col={}: got=0x{got:08X}, expected=0x{want:08X}",
                    case.name, r, c
                ));
            }

            log_line(&mut pattern_log, &format!("pattern={} loaded", case.name))?;

            for step in 1..=case.steps {
                expected = oracle_step(&expected);
                log_grid(
                    &mut pattern_log,
                    &format!("pattern={} expected step={}:", case.name, step),
                    &expected,
                )?;

                if case.toroidal_check {
                    let expected_edges = edge_alive_counts(&expected);
                    if first_expected_wrap_step.is_none()
                        && has_top_or_left_activity(expected_edges)
                    {
                        first_expected_wrap_step = Some(step);
                    }
                    log_line(
                        &mut pattern_log,
                        &format!(
                            "pattern={} expected step={} toroidal edges: top={}, bottom={}, left={}, right={}",
                            case.name,
                            step,
                            expected_edges.top,
                            expected_edges.bottom,
                            expected_edges.left,
                            expected_edges.right
                        ),
                    )?;
                }

                try_flush_input(&mut *port);
                send_step_once(&mut *port)?;

                let actual = await_matching_frame(
                    &mut *port,
                    &mut pattern_log,
                    &format!("pattern={} step={}", case.name, step),
                    &expected,
                    Duration::from_secs(3),
                )?;
                if let Some((r, c, got, want)) = first_diff(&actual, &expected) {
                    log_grid(&mut pattern_log, "actual grid:", &actual)?;
                    log_grid(&mut pattern_log, "expected grid:", &expected)?;
                    return Err(format!(
                        "pattern={} step={} mismatch at row={}, col={}: got=0x{got:08X}, expected=0x{want:08X}",
                        case.name, step, r, c
                    ));
                }

                if case.toroidal_check {
                    let actual_edges = edge_alive_counts(&actual);
                    if first_actual_wrap_step.is_none() && has_top_or_left_activity(actual_edges) {
                        first_actual_wrap_step = Some(step);
                    }
                    log_line(
                        &mut pattern_log,
                        &format!(
                            "pattern={} actual step={} toroidal edges: top={}, bottom={}, left={}, right={}",
                            case.name,
                            step,
                            actual_edges.top,
                            actual_edges.bottom,
                            actual_edges.left,
                            actual_edges.right
                        ),
                    )?;
                }

                log_line(
                    &mut pattern_log,
                    &format!("pattern={} step={} OK", case.name, step),
                )?;
            }

            if case.toroidal_check {
                let expected_wrap = first_expected_wrap_step.ok_or_else(|| {
                    format!(
                        "pattern={} toroidal-check failed: oracle never produced top/left edge activity within {} steps",
                        case.name, case.steps
                    )
                })?;
                let actual_wrap = first_actual_wrap_step.ok_or_else(|| {
                    format!(
                        "pattern={} toroidal-check failed: device never produced top/left edge activity within {} steps",
                        case.name, case.steps
                    )
                })?;

                log_line(
                    &mut pattern_log,
                    &format!(
                        "pattern={} toroidal-check wrap detected: expected_step={}, actual_step={}",
                        case.name, expected_wrap, actual_wrap
                    ),
                )?;

                if actual_wrap != expected_wrap {
                    return Err(format!(
                        "pattern={} toroidal-check mismatch: expected first wrap activity at step {}, got step {}",
                        case.name, expected_wrap, actual_wrap
                    ));
                }
            }

            Ok(())
        })();

        match pattern_result {
            Ok(()) => {
                let pass = format!("pattern={} PASS", case.name);
                log_line(&mut pattern_log, &pass)?;
                summary_results.push((case.name.to_string(), true));
                summary_lines.push(pass.clone());
                passes.push(pass);
            }
            Err(err) => {
                let failure = format!("pattern={} FAIL: {err}", case.name);
                log_line(&mut pattern_log, &failure)?;
                summary_results.push((case.name.to_string(), false));
                summary_lines.push(failure.clone());
                failures.push(failure);
            }
        }

        pattern_log
            .flush()
            .map_err(|e| format!("failed to flush pattern log file: {e}"))?;
    }

    if failures.is_empty() {
        log_line(&mut init_log, "HIL result: PASS")?;
        summary_lines.push(format!(
            "HIL result: PASS ({} passed, {} failed)",
            passes.len(),
            failures.len()
        ));
    } else {
        log_line(&mut init_log, "HIL result: FAIL")?;
        summary_lines.push(format!(
            "HIL result: FAIL ({} passed, {} failed)",
            passes.len(),
            failures.len()
        ));
        for failure in &failures {
            log_line(&mut init_log, failure)?;
        }
    }

    let total_tests = summary_results.len();
    let passed_tests = summary_results.iter().filter(|(_, passed)| *passed).count();
    let failed_tests = total_tests.saturating_sub(passed_tests);

    file_line(&mut summary_log, &format!("Total tests: {}", total_tests))?;
    file_line(&mut summary_log, &format!("Passed: {}", passed_tests))?;
    file_line(&mut summary_log, &format!("Failed: {}", failed_tests))?;
    for (name, passed) in &summary_results {
        file_line(
            &mut summary_log,
            &format!("{}: {}", name, if *passed { "PASS" } else { "FAIL" }),
        )?;
    }
    file_line(&mut summary_log, "")?;

    for line in &summary_lines {
        file_line(&mut summary_log, line)?;
    }

    init_log
        .flush()
        .map_err(|e| format!("failed to flush grid init log file: {e}"))?;
    summary_log
        .flush()
        .map_err(|e| format!("failed to flush summary log file: {e}"))?;

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} HIL check(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        ))
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("HIL result: FAIL: {}", e);
        std::process::exit(1);
    }
}
