use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
    time::Instant,
};
use tower_http::cors::{Any, CorsLayer};

const ROWS: usize = 3;
const COLS: usize = 3;
const NUM_CELLS: usize = 9;
const VALUES: [u32; 10] = [0, 2, 4, 8, 16, 32, 64, 128, 256, 512];
const NUM_VALUES: u64 = 10;
const WIN_VALUE: u32 = 1024;
// To win, we need two 512s (sum >= 1024). Max realistic sum with path to victory:
// 512+512+256+128+64+32+16+8+4 = 1532, round up for safety
const MAX_SUM: u32 = 1534;

const DATA_DIR: &str = "probability_data";

type Grid = [u32; NUM_CELLS];

// Fixed-point probability: value represents x / 2^32
// So 0 = 0.0, u32::MAX = 0.999999999767..., and we treat 1.0 specially
type Prob = u32;

const PROB_ONE: Prob = u32::MAX;
const PROB_ZERO: Prob = 0;

// Convert f32 to fixed-point
fn f32_to_prob(f: f32) -> Prob {
    if f >= 1.0 {
        PROB_ONE
    } else if f <= 0.0 {
        PROB_ZERO
    } else {
        ((f as f64) * (u32::MAX as f64 + 1.0)) as Prob
    }
}

// Convert fixed-point to f32 (for JSON API)
fn prob_to_f32(p: Prob) -> f32 {
    (p as f64 / (u32::MAX as f64 + 1.0)) as f32
}

// Multiply two probabilities: (a/2^32) * (b/2^32) = (a*b)/2^64, then scale back to /2^32
fn prob_mul(a: Prob, b: Prob) -> Prob {
    (((a as u64) * (b as u64)) >> 32) as Prob
}

// Divide probability by small integer
fn prob_div(p: Prob, divisor: u32) -> Prob {
    (p as u64 / divisor as u64) as Prob
}

// Add probabilities (caller must ensure no overflow)
fn prob_add(a: Prob, b: Prob) -> Prob {
    a.saturating_add(b)
}

fn val_to_idx(val: u32) -> u64 {
    match val {
        0 => 0, 2 => 1, 4 => 2, 8 => 3, 16 => 4,
        32 => 5, 64 => 6, 128 => 7, 256 => 8, 512 => 9,
        _ => 0,
    }
}

fn idx_to_val(idx: u64) -> u32 {
    VALUES[idx as usize]
}

fn encode_state(grid: &Grid) -> u64 {
    let mut code: u64 = 0;
    let mut multiplier: u64 = 1;
    for &val in grid.iter() {
        code += val_to_idx(val) * multiplier;
        multiplier *= NUM_VALUES;
    }
    code
}

fn decode_state(mut code: u64) -> Grid {
    let mut grid = [0u32; NUM_CELLS];
    for i in 0..NUM_CELLS {
        grid[i] = idx_to_val(code % NUM_VALUES);
        code /= NUM_VALUES;
    }
    grid
}

fn rotate90(grid: &Grid) -> Grid {
    [grid[6], grid[3], grid[0], grid[7], grid[4], grid[1], grid[8], grid[5], grid[2]]
}

fn reflect_h(grid: &Grid) -> Grid {
    [grid[2], grid[1], grid[0], grid[5], grid[4], grid[3], grid[8], grid[7], grid[6]]
}

fn get_symmetries(grid: &Grid) -> [Grid; 8] {
    let r1 = rotate90(grid);
    let r2 = rotate90(&r1);
    let r3 = rotate90(&r2);
    [
        *grid,
        reflect_h(grid),
        r1,
        reflect_h(&r1),
        r2,
        reflect_h(&r2),
        r3,
        reflect_h(&r3),
    ]
}

fn get_canonical(grid: &Grid) -> u64 {
    get_symmetries(grid)
        .iter()
        .map(|g| encode_state(g))
        .min()
        .unwrap()
}

fn grid_to_2d(grid: &Grid) -> [[u32; COLS]; ROWS] {
    [
        [grid[0], grid[1], grid[2]],
        [grid[3], grid[4], grid[5]],
        [grid[6], grid[7], grid[8]],
    ]
}

fn grid_from_2d(g2d: &[[u32; COLS]; ROWS]) -> Grid {
    [
        g2d[0][0], g2d[0][1], g2d[0][2],
        g2d[1][0], g2d[1][1], g2d[1][2],
        g2d[2][0], g2d[2][1], g2d[2][2],
    ]
}

fn slide_row(row: &[u32; 3]) -> ([u32; 3], bool, bool) {
    let mut arr: Vec<u32> = row.iter().copied().filter(|&x| x != 0).collect();
    let mut won = false;

    let mut i = 0;
    while i < arr.len().saturating_sub(1) {
        if arr[i] == arr[i + 1] {
            arr[i] *= 2;
            if arr[i] >= WIN_VALUE {
                won = true;
            }
            arr.remove(i + 1);
        }
        i += 1;
    }

    while arr.len() < 3 {
        arr.push(0);
    }

    let result = [arr[0], arr[1], arr[2]];
    let moved = result != *row;
    (result, moved, won)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up, Down, Left, Right,
}

fn simulate_move(grid: &Grid, dir: Direction) -> Option<(Grid, bool)> {
    let mut g2d = grid_to_2d(grid);
    let mut any_moved = false;
    let mut won = false;

    match dir {
        Direction::Left => {
            for r in 0..3 {
                let (result, moved, w) = slide_row(&g2d[r]);
                if moved { any_moved = true; }
                if w { won = true; }
                g2d[r] = result;
            }
        }
        Direction::Right => {
            for r in 0..3 {
                let mut row = g2d[r];
                row.reverse();
                let (mut result, moved, w) = slide_row(&row);
                result.reverse();
                if moved { any_moved = true; }
                if w { won = true; }
                g2d[r] = result;
            }
        }
        Direction::Up => {
            for c in 0..3 {
                let col = [g2d[0][c], g2d[1][c], g2d[2][c]];
                let (result, moved, w) = slide_row(&col);
                if moved { any_moved = true; }
                if w { won = true; }
                g2d[0][c] = result[0];
                g2d[1][c] = result[1];
                g2d[2][c] = result[2];
            }
        }
        Direction::Down => {
            for c in 0..3 {
                let mut col = [g2d[2][c], g2d[1][c], g2d[0][c]];
                let (mut result, moved, w) = slide_row(&col);
                result.reverse();
                if moved { any_moved = true; }
                if w { won = true; }
                g2d[0][c] = result[0];
                g2d[1][c] = result[1];
                g2d[2][c] = result[2];
            }
        }
    }

    if any_moved {
        Some((grid_from_2d(&g2d), won))
    } else {
        None
    }
}

fn can_win(grid: &Grid) -> bool {
    [Direction::Up, Direction::Down, Direction::Left, Direction::Right]
        .iter()
        .any(|&dir| simulate_move(grid, dir).map(|(_, won)| won).unwrap_or(false))
}

fn is_game_over(grid: &Grid) -> bool {
    if grid.iter().any(|&v| v == 0) {
        return false;
    }

    let g2d = grid_to_2d(grid);

    for r in 0..3 {
        for c in 0..3 {
            if c < 2 && g2d[r][c] == g2d[r][c + 1] { return false; }
            if r < 2 && g2d[r][c] == g2d[r + 1][c] { return false; }
        }
    }

    true
}

fn get_empty_cells(grid: &Grid) -> Vec<usize> {
    grid.iter()
        .enumerate()
        .filter(|(_, &v)| v == 0)
        .map(|(i, _)| i)
        .collect()
}

fn has_winning_setup(grid: &Grid) -> bool {
    let vals: Vec<u32> = grid.iter().copied().filter(|&v| v > 0).collect();
    [512, 256, 128, 64, 32, 16, 8, 4].iter().all(|&v| vals.contains(&v))
}

fn grid_sum(grid: &Grid) -> u32 {
    grid.iter().sum()
}

/// Generate all multisets of 9 values from VALUES that sum to target_sum
fn generate_multisets_with_sum(target_sum: u32) -> Vec<[u32; NUM_CELLS]> {
    let mut results = Vec::new();

    // Recursive generator that builds multisets in non-decreasing order (by value index)
    fn generate(
        cell_idx: usize,
        min_val_idx: usize,  // minimum value index to use (ensures non-decreasing order)
        current_sum: u32,
        target: u32,
        current: &mut [u32; NUM_CELLS],
        results: &mut Vec<[u32; NUM_CELLS]>,
    ) {
        if cell_idx == NUM_CELLS {
            if current_sum == target {
                results.push(*current);
            }
            return;
        }

        let remaining = NUM_CELLS - cell_idx;

        for val_idx in min_val_idx..VALUES.len() {
            let val = VALUES[val_idx];
            let new_sum = current_sum + val;

            // Pruning: if we're already over, stop (values are increasing)
            if new_sum > target {
                break;
            }

            // Pruning: can we reach target with remaining cells using max value?
            let max_remaining = (remaining - 1) as u32 * 512;
            if new_sum + max_remaining < target {
                continue;
            }

            // Pruning: if we fill remaining cells with this same value, would we overshoot?
            // (This helps prune when val is large)
            let min_remaining = (remaining - 1) as u32 * val;
            if new_sum + min_remaining > target {
                // Can't use this value or any larger value for all remaining cells
                // But we might still use a mix, so don't break, just continue
            }

            current[cell_idx] = val;
            generate(cell_idx + 1, val_idx, new_sum, target, current, results);
        }
    }

    let mut current = [0u32; NUM_CELLS];
    generate(0, 0, 0, target_sum, &mut current, &mut results);
    results
}

/// Generate all unique permutations of a multiset
fn generate_permutations(multiset: &[u32; NUM_CELLS]) -> Vec<Grid> {
    let mut results = Vec::new();
    let mut current = *multiset;

    // Sort to start with lexicographically smallest permutation
    current.sort();

    loop {
        results.push(current);

        // Find next permutation (standard algorithm)
        // Find largest i such that current[i] < current[i+1]
        let mut i = None;
        for k in (0..NUM_CELLS-1).rev() {
            if current[k] < current[k + 1] {
                i = Some(k);
                break;
            }
        }

        let i = match i {
            Some(i) => i,
            None => break,  // No more permutations
        };

        // Find largest j > i such that current[i] < current[j]
        let mut j = i + 1;
        for k in (i+1..NUM_CELLS).rev() {
            if current[i] < current[k] {
                j = k;
                break;
            }
        }

        // Swap i and j
        current.swap(i, j);

        // Reverse from i+1 to end
        current[i+1..].reverse();
    }

    results
}

/// Generate all grids with a specific sum, returning only canonical representatives
fn generate_grids_with_sum(target_sum: u32) -> Vec<(u64, Grid)> {
    let mut seen_canonical: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut results = Vec::new();

    // First, find all multisets that sum to target
    let multisets = generate_multisets_with_sum(target_sum);

    // For each multiset, generate all permutations and canonicalize
    for multiset in &multisets {
        let permutations = generate_permutations(multiset);
        for grid in permutations {
            let canonical = get_canonical(&grid);
            if seen_canonical.insert(canonical) {
                results.push((canonical, grid));
            }
        }
    }

    results
}

/// Load probabilities from a file for a given sum
fn load_probs_for_sum(sum: u32) -> HashMap<u64, Prob> {
    let path = format!("{}/sum_{:04}.dat", DATA_DIR, sum);
    let mut probs = HashMap::new();

    if let Ok(file) = File::open(&path) {
        let reader = BufReader::new(file);
        for line in reader.lines() {
            if let Ok(line) = line {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() == 2 {
                    if let (Ok(code), Ok(prob)) = (parts[0].parse::<u64>(), parts[1].parse::<Prob>()) {
                        probs.insert(code, prob);
                    }
                }
            }
        }
    }

    probs
}

/// Save probabilities to a file for a given sum
fn save_probs_for_sum(sum: u32, probs: &HashMap<u64, Prob>) -> std::io::Result<()> {
    let path = format!("{}/sum_{:04}.dat", DATA_DIR, sum);
    let file = File::create(&path)?;
    let mut writer = BufWriter::new(file);

    for (&code, &prob) in probs {
        writeln!(writer, "{},{}", code, prob)?;
    }

    writer.flush()?;
    Ok(())
}

/// Check if probabilities for a sum have already been computed
fn sum_already_computed(sum: u32) -> bool {
    let path = format!("{}/sum_{:04}.dat", DATA_DIR, sum);
    Path::new(&path).exists()
}

// Spawn probabilities as fixed-point (90% for 2, 10% for 4)
const SPAWN_PROB_2: Prob = 3865470566; // 0.9 * 2^32
const SPAWN_PROB_4: Prob = 429496730;  // 0.1 * 2^32

/// Compute probabilities incrementally, only keeping necessary sums in memory
fn compute_probabilities_incremental() {
    let start = Instant::now();

    fs::create_dir_all(DATA_DIR).expect("Failed to create data directory");

    // Find where to resume from
    let mut start_sum = MAX_SUM;
    while start_sum > 0 && sum_already_computed(start_sum) {
        start_sum -= 2;
    }

    if start_sum < MAX_SUM {
        println!("Resuming from sum {}...", start_sum);
    }

    // We need probs for sum+2 and sum+4 to compute sum
    // Process from high to low
    let mut probs_plus_2: HashMap<u64, Prob> = if start_sum < MAX_SUM {
        load_probs_for_sum(start_sum + 2)
    } else {
        HashMap::new()
    };

    let mut probs_plus_4: HashMap<u64, Prob> = if start_sum < MAX_SUM - 2 {
        load_probs_for_sum(start_sum + 4)
    } else {
        HashMap::new()
    };

    // Process sums from high to low (only even sums matter since all values are even)
    let mut current_sum = start_sum;

    while current_sum <= MAX_SUM {
        if sum_already_computed(current_sum) {
            println!("Sum {} already computed, skipping...", current_sum);
            current_sum = current_sum.saturating_sub(2);
            if current_sum == 0 && sum_already_computed(0) {
                break;
            }
            continue;
        }

        let grids = generate_grids_with_sum(current_sum);
        println!("Processing sum {}: {} unique canonical states...", current_sum, grids.len());

        let mut current_probs: HashMap<u64, Prob> = HashMap::new();

        for (canonical, grid) in &grids {
            // Check terminal conditions
            if can_win(grid) {
                current_probs.insert(*canonical, PROB_ONE);
                continue;
            }

            if is_game_over(grid) {
                current_probs.insert(*canonical, PROB_ZERO);
                continue;
            }

            // Try all moves, take the best
            let mut best_prob: Prob = PROB_ZERO;

            for dir in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
                if let Some((new_grid, won)) = simulate_move(grid, dir) {
                    if won {
                        best_prob = PROB_ONE;
                        break;
                    }

                    let empty_cells = get_empty_cells(&new_grid);
                    if empty_cells.is_empty() {
                        continue;
                    }

                    let win_setup = has_winning_setup(&new_grid);
                    let spawn_options: &[(u32, Prob)] = if win_setup {
                        &[(4, PROB_ONE)]
                    } else {
                        &[(2, SPAWN_PROB_2), (4, SPAWN_PROB_4)]
                    };

                    let mut expected_prob: Prob = PROB_ZERO;
                    let num_empty = empty_cells.len() as u32;

                    for &cell_idx in &empty_cells {
                        for &(spawn_val, p) in spawn_options {
                            let mut copy = new_grid;
                            copy[cell_idx] = spawn_val;
                            let new_canonical = get_canonical(&copy);
                            let new_sum = grid_sum(&copy);

                            // The new sum should be current_sum + spawn_val
                            let state_prob = if new_sum == current_sum + 2 {
                                probs_plus_2.get(&new_canonical).copied().unwrap_or(PROB_ZERO)
                            } else if new_sum == current_sum + 4 {
                                probs_plus_4.get(&new_canonical).copied().unwrap_or(PROB_ZERO)
                            } else {
                                PROB_ZERO
                            };

                            // expected_prob += p * state_prob / num_empty
                            expected_prob = prob_add(expected_prob, prob_div(prob_mul(p, state_prob), num_empty));
                        }
                    }

                    if expected_prob > best_prob {
                        best_prob = expected_prob;
                    }
                }
            }

            current_probs.insert(*canonical, best_prob);
        }

        // Save current sum to disk
        save_probs_for_sum(current_sum, &current_probs).expect("Failed to save probabilities");
        println!("  Saved {} states for sum {}", current_probs.len(), current_sum);

        // Shift the windows
        probs_plus_4 = probs_plus_2;
        probs_plus_2 = current_probs;

        if current_sum == 0 {
            break;
        }
        current_sum -= 2;
    }

    println!("Computation complete in {:?}", start.elapsed());
}

/// Load all probabilities from disk
fn load_all_probabilities() -> HashMap<u64, Prob> {
    println!("Loading probabilities from disk...");
    let start = Instant::now();

    let mut all_probs = HashMap::new();
    let total_files = (MAX_SUM / 2 + 1) as usize;
    let mut files_loaded = 0;

    for sum in (0..=MAX_SUM).step_by(2) {
        let probs = load_probs_for_sum(sum);
        all_probs.extend(probs);
        files_loaded += 1;

        if files_loaded % 50 == 0 || files_loaded == total_files {
            print!("\r  Loaded {}/{} files ({} states)...", files_loaded, total_files, all_probs.len());
            std::io::Write::flush(&mut std::io::stdout()).ok();
        }
    }
    println!();

    println!("Loaded {} total states in {:?}", all_probs.len(), start.elapsed());
    all_probs
}

/// Check if all probability data exists
fn all_data_computed() -> bool {
    for sum in (0..=MAX_SUM).step_by(2) {
        if !sum_already_computed(sum) {
            return false;
        }
    }
    true
}

#[derive(Clone)]
struct AppState {
    probs: Arc<HashMap<u64, Prob>>,
}

#[derive(Deserialize)]
struct ProbQuery {
    state: String,
}

#[derive(Serialize)]
struct ProbResponse {
    probability: f32,
    canonical_code: u64,
}

#[derive(Serialize)]
struct MoveProbs {
    up: Option<f32>,
    down: Option<f32>,
    left: Option<f32>,
    right: Option<f32>,
}

#[derive(Deserialize)]
struct MovesQuery {
    state: String,
}

async fn get_probability(
    State(state): State<AppState>,
    Query(query): Query<ProbQuery>,
) -> impl IntoResponse {
    let values: Result<Vec<u32>, _> = query
        .state
        .split(',')
        .map(|s| s.trim().parse::<u32>())
        .collect();

    match values {
        Ok(vals) if vals.len() == 9 => {
            let mut grid = [0u32; 9];
            for (i, v) in vals.into_iter().enumerate() {
                grid[i] = v;
            }
            let canonical = get_canonical(&grid);
            let prob = state.probs.get(&canonical).copied().unwrap_or(PROB_ZERO);
            Json(ProbResponse {
                probability: prob_to_f32(prob),
                canonical_code: canonical,
            })
        }
        _ => Json(ProbResponse {
            probability: 0.0,
            canonical_code: 0,
        }),
    }
}

fn get_move_expected_prob(grid: &Grid, dir: Direction, probs: &HashMap<u64, Prob>) -> Option<f32> {
    let (new_grid, won) = simulate_move(grid, dir)?;

    if won {
        return Some(1.0);
    }

    let empty_cells = get_empty_cells(&new_grid);
    if empty_cells.is_empty() {
        return Some(0.0);
    }

    let win_setup = has_winning_setup(&new_grid);
    let spawn_options: &[(u32, Prob)] = if win_setup {
        &[(4, PROB_ONE)]
    } else {
        &[(2, SPAWN_PROB_2), (4, SPAWN_PROB_4)]
    };

    let mut expected_prob: Prob = PROB_ZERO;
    let num_empty = empty_cells.len() as u32;

    for &cell_idx in &empty_cells {
        for &(val, p) in spawn_options {
            let mut copy = new_grid;
            copy[cell_idx] = val;
            let new_canon = get_canonical(&copy);
            let state_prob = probs.get(&new_canon).copied().unwrap_or(PROB_ZERO);
            expected_prob = prob_add(expected_prob, prob_div(prob_mul(p, state_prob), num_empty));
        }
    }

    Some(prob_to_f32(expected_prob))
}

async fn get_move_probs(
    State(state): State<AppState>,
    Query(query): Query<MovesQuery>,
) -> impl IntoResponse {
    let values: Result<Vec<u32>, _> = query
        .state
        .split(',')
        .map(|s| s.trim().parse::<u32>())
        .collect();

    match values {
        Ok(vals) if vals.len() == 9 => {
            let mut grid = [0u32; 9];
            for (i, v) in vals.into_iter().enumerate() {
                grid[i] = v;
            }

            Json(MoveProbs {
                up: get_move_expected_prob(&grid, Direction::Up, &state.probs),
                down: get_move_expected_prob(&grid, Direction::Down, &state.probs),
                left: get_move_expected_prob(&grid, Direction::Left, &state.probs),
                right: get_move_expected_prob(&grid, Direction::Right, &state.probs),
            })
        }
        _ => Json(MoveProbs {
            up: None,
            down: None,
            left: None,
            right: None,
        }),
    }
}

async fn serve_html() -> impl IntoResponse {
    // Read from disk each time for development (no caching)
    match fs::read_to_string("2048-3x3-client.html") {
        Ok(content) => Html(content),
        Err(_) => Html("<h1>Error: Could not read 2048-3x3-client.html</h1>".to_string()),
    }
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

fn print_usage() {
    eprintln!("Usage: game2048-server [command]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  compute    Compute probabilities and save to disk (memory-efficient)");
    eprintln!("  serve      Start the web server (loads precomputed data)");
    eprintln!("  (none)     Auto-detect: compute if needed, then serve");
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    let command = args.get(1).map(|s| s.as_str());

    match command {
        Some("compute") => {
            println!("=== Computing probabilities (memory-efficient mode) ===");
            compute_probabilities_incremental();
            println!("Done! Run with 'serve' to start the server.");
        }
        Some("serve") => {
            if !all_data_computed() {
                eprintln!("Error: Probability data not found. Run with 'compute' first.");
                std::process::exit(1);
            }

            let probs = load_all_probabilities();
            start_server(probs).await;
        }
        Some("--help") | Some("-h") => {
            print_usage();
        }
        Some(unknown) => {
            eprintln!("Unknown command: {}", unknown);
            print_usage();
            std::process::exit(1);
        }
        None => {
            // Auto mode: compute if needed, then serve
            if !all_data_computed() {
                println!("Probability data not found. Computing...");
                compute_probabilities_incremental();
            }

            let probs = load_all_probabilities();
            start_server(probs).await;
        }
    }
}

async fn start_server(probs: HashMap<u64, Prob>) {
    println!("Starting server with {} states loaded", probs.len());

    let state = AppState {
        probs: Arc::new(probs),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(serve_html))
        .route("/health", get(health_check))
        .route("/api/prob", get(get_probability))
        .route("/api/moves", get(get_move_probs))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("Server running at http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}
