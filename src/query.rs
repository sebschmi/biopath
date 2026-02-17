use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    str::FromStr,
    time::Instant,
};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::{DirectedNodeIndex, GraphIndexInteger},
    io::gfa1::{PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use indicatif::ProgressBar;
use itertools::Itertools;
use log::{LevelFilter, info, warn};
use spqr_shortest_path_index::{
    dijkstra::GfaDijkstra,
    location::{GfaLocation, GfaNodeOffset},
    location_index::{multi::MultiGfaLocationIndex, single::SingleGfaLocationIndex},
    path::OptionalGfaPathLength,
    spqr_decomposition_overlay::{SPQRDecompositionOverlay, dijkstra::OverlayDijkstra},
};
use spqr_tree::{decomposition::SPQRDecomposition, graph::StaticGraph};

use crate::io_util::{
    open_optionally_compressed_file, read_optionally_compressed_file,
    write_optionally_compressed_file,
};

#[derive(Parser)]
pub struct Cli {
    #[clap(long, default_value = "info")]
    pub(crate) log_level: LevelFilter,

    /// The GFA file containing the graph to index.
    #[clap(long)]
    graph_gfa_in: PathBuf,

    /// The SPQR decomposition in plain text format.
    #[clap(long, requires = "index_in")]
    spqr_in: Option<PathBuf>,

    /// The index file.
    /// If no index is given, then the queries will be run with Dijkstra on the input graph.
    #[clap(long, requires = "spqr_in")]
    index_in: Option<PathBuf>,

    /// A tab-separated file containing the queries to run.
    /// The columns are `source_node_id`, `source_orientation`, `source_offset`, `target_node_id`, `target_orientation`, `target_offset`.
    /// The last three columns can be repeated to specify multiple target locations for the same source.
    #[clap(long)]
    query_in: PathBuf,

    /// The output file for the query results.
    /// Contains a copy of the input rows with and additional column for the `distance` for each target.
    #[clap(long)]
    query_out: PathBuf,
}

struct Query<IndexType> {
    source: GfaLocation<IndexType>,
    targets: Vec<GfaLocation<IndexType>>,
    distances: Vec<OptionalGfaPathLength<IndexType>>,
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.index_in.is_none() {
        warn!(
            "No index file provided, running queries with Dijkstra on the input graph. This may be very slow for large graphs."
        );
        return run_without_index::<u64>(cli);
    }

    // Read word size from index file first.
    let mut index_file_reader = BufReader::new(
        open_optionally_compressed_file(cli.index_in.as_ref().unwrap())
            .with_context(|| format!("Failed to open index file {:?}", cli.index_in))?,
    );
    let mut word_size_bytes = [0u8; 1];
    index_file_reader
        .read_exact(&mut word_size_bytes)
        .with_context(|| format!("Failed to read index header from file {:?}", cli.index_in))?;
    let word_size = word_size_bytes[0];

    info!(
        "Discovered word size {} bits from index file header",
        word_size
    );

    match word_size {
        8 => run_with_word_size::<u8>(cli, index_file_reader),
        16 => run_with_word_size::<u16>(cli, index_file_reader),
        32 => run_with_word_size::<u32>(cli, index_file_reader),
        64 => run_with_word_size::<u64>(cli, index_file_reader),
        _ => anyhow::bail!(
            "Unsupported word size: {}. Supported are 8, 16, 32 and 64.",
            word_size
        ),
    }
}

fn run_with_word_size<IndexType: GraphIndexInteger + FromStr>(
    cli: Cli,
    index_file_reader: impl BufRead,
) -> anyhow::Result<()>
where
    <IndexType as FromStr>::Err: std::error::Error + Send + Sync + 'static,
{
    info!("Reading graph from GFA file {:?}", cli.graph_gfa_in);
    let graph = read_optionally_compressed_file(&cli.graph_gfa_in, |reader| {
        BidirectedAdjacencyArray::<IndexType, PlainGfaNodeData, PlainGfaEdgeData>::read_gfa1(reader)
            .with_context(|| format!("Failed to parse GFA file {:?}", cli.graph_gfa_in))
    })
    .with_context(|| format!("Failed to read GFA file: {:?}", cli.graph_gfa_in))?;
    info!(
        "Graph has {} nodes and {} edges",
        graph.node_count(),
        graph.edge_count(),
    );

    info!("Building node name index");
    let node_name_index: HashMap<_, _> = graph
        .node_indices()
        .map(|node_index| (graph.node_name(node_index), node_index))
        .collect();

    info!("Reading SPQR decomposition from file {:?}", cli.spqr_in);
    let spqr_decomposition =
        read_optionally_compressed_file(cli.spqr_in.as_ref().unwrap(), |reader| {
            SPQRDecomposition::read_plain_spqr(&graph, reader).with_context(|| {
                format!("Failed to parse SPQR decomposition file {:?}", cli.spqr_in)
            })
        })
        .with_context(|| format!("Failed to read SPQR file: {:?}", cli.spqr_in))?;

    info!("Reading index from file {:?}", cli.index_in);
    let overlay =
        SPQRDecompositionOverlay::read_binary(&graph, &spqr_decomposition, index_file_reader)
            .with_context(|| format!("Failed to read index file: {:?}", cli.index_in))?;

    info!("Reading queries from file {:?}", cli.query_in);
    let mut queries = read_optionally_compressed_file(&cli.query_in, |reader| {
        let mut queries = Vec::new();
        for line in reader.lines() {
            let line = line.with_context(|| {
                format!("Failed to read line from query file: {:?}", cli.query_in)
            })?;

            let mut source = None;
            let mut targets = Vec::new();

            let columns = line.trim().split('\t').collect_vec();
            for column in columns.chunks(3) {
                if column.len() != 3 {
                    anyhow::bail!(
                        "Invalid query line in file {:?}: expected a number of columns that is divisible by 3, got {}",
                        cli.query_in,
                        columns.len()
                    );
                }

                let node_name = column[0];
                let forward = match column[1] {
                    "+" => true,
                    "-" => false,
                    _ => anyhow::bail!(
                        "Invalid query line in file {:?}: expected orientation to be either '+' or '-', got '{}'",
                        cli.query_in,
                        column[1]
                    ),
                };
                let offset = column[2].parse::<IndexType>().map(GfaNodeOffset::from_raw).with_context(|| {
                    format!(
                        "Invalid query line in file {:?}: failed to parse offset '{}'",
                        cli.query_in,
                        column[2],
                    )
                })?;

                let location = GfaLocation::new(DirectedNodeIndex::from_bidirected(node_name_index[node_name], forward), offset);
                if source.is_none() {
                    source = Some(location);
                } else {
                    targets.push(location);
                }
            }

            if source.is_none() || targets.is_empty() {
                anyhow::bail!(
                    "Invalid query line in file {:?}: expected at least one source and one target location, got line '{}'",
                    cli.query_in,
                    line
                );
            }

            queries.push(Query { source: source.unwrap(), targets, distances: Vec::new() });
        }

        Ok(queries)
    })
    .with_context(|| format!("Failed to read query file: {:?}", cli.query_in))?;

    info!("Initialising overlay Dijkstra data structures");
    let mut dijkstra = OverlayDijkstra::new(&overlay);

    info!("Executing queries");
    let progress_bar =
        ProgressBar::new(queries.len().try_into().unwrap()).with_message("Executing queries");
    let start_time = Instant::now();

    for query in &mut queries {
        let paths = if query.targets.len() == 1 {
            dijkstra.shortest_paths(
                query.source,
                &SingleGfaLocationIndex::new_target(query.targets[0]),
            )
        } else {
            dijkstra.shortest_paths(
                query.source,
                &MultiGfaLocationIndex::new_targets(&graph, query.targets.iter().copied()),
            )
        };
        query.distances = query
            .targets
            .iter()
            .map(|&target| paths.get(&target).map(|path| path.length()).into())
            .collect();

        progress_bar.inc(1);
    }

    let end_time = Instant::now();
    progress_bar.finish_and_clear();

    info!(
        "Finished executing {} queries in {:.2?} ({:.0}µs per query)",
        queries.len(),
        end_time - start_time,
        (end_time - start_time).as_secs_f64() / queries.len() as f64 * 1_000_000.0
    );

    info!("Writing query results to file {:?}", cli.query_out);
    write_optionally_compressed_file(&cli.query_out, |writer| {
        for query in &queries {
            write!(
                writer,
                "{}\t{}\t{}",
                graph.node_name(query.source.node().into_bidirected()),
                query.source.offset(),
                if query.source.node().is_forward() {
                    "+"
                } else {
                    "-"
                },
            )?;

            for (target, distance) in query.targets.iter().zip(&query.distances) {
                write!(
                    writer,
                    "{}\t{}\t{}\t{}",
                    graph.node_name(target.node().into_bidirected()),
                    target.offset(),
                    if target.node().is_forward() { "+" } else { "-" },
                    distance
                        .into_option()
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "None".to_string()),
                )?;
            }

            writeln!(writer)?;
        }
        Ok(())
    })
    .with_context(|| format!("Failed to write query results to file: {:?}", cli.query_out))?;

    Ok(())
}

fn run_without_index<IndexType: GraphIndexInteger + FromStr>(cli: Cli) -> anyhow::Result<()>
where
    <IndexType as FromStr>::Err: std::error::Error + Send + Sync + 'static,
{
    info!("Reading graph from GFA file {:?}", cli.graph_gfa_in);
    let graph = read_optionally_compressed_file(&cli.graph_gfa_in, |reader| {
        BidirectedAdjacencyArray::<IndexType, PlainGfaNodeData, PlainGfaEdgeData>::read_gfa1(reader)
            .with_context(|| format!("Failed to parse GFA file {:?}", cli.graph_gfa_in))
    })
    .with_context(|| format!("Failed to read GFA file: {:?}", cli.graph_gfa_in))?;
    info!(
        "Graph has {} nodes and {} edges",
        graph.node_count(),
        graph.edge_count(),
    );

    info!("Building node name index");
    let node_name_index: HashMap<_, _> = graph
        .node_indices()
        .map(|node_index| (graph.node_name(node_index), node_index))
        .collect();

    info!("Reading queries from file {:?}", cli.query_in);
    let mut queries = read_optionally_compressed_file(&cli.query_in, |reader| {
        let mut queries = Vec::new();
        for line in reader.lines() {
            let line = line.with_context(|| {
                format!("Failed to read line from query file: {:?}", cli.query_in)
            })?;

            let mut source = None;
            let mut targets = Vec::new();

            let columns = line.trim().split('\t').collect_vec();
            for column in columns.chunks(3) {
                if column.len() != 3 {
                    anyhow::bail!(
                        "Invalid query line in file {:?}: expected a number of columns that is divisible by 3, got {}",
                        cli.query_in,
                        columns.len()
                    );
                }

                let node_name = column[0];
                let forward = match column[1] {
                    "+" => true,
                    "-" => false,
                    _ => anyhow::bail!(
                        "Invalid query line in file {:?}: expected orientation to be either '+' or '-', got '{}'",
                        cli.query_in,
                        column[1]
                    ),
                };
                let offset = column[2].parse::<IndexType>().map(GfaNodeOffset::from_raw).with_context(|| {
                    format!(
                        "Invalid query line in file {:?}: failed to parse offset '{}'",
                        cli.query_in,
                        column[2],
                    )
                })?;

                let location = GfaLocation::new(DirectedNodeIndex::from_bidirected(node_name_index[node_name], forward), offset);
                if source.is_none() {
                    source = Some(location);
                } else {
                    targets.push(location);
                }
            }

            if source.is_none() || targets.is_empty() {
                anyhow::bail!(
                    "Invalid query line in file {:?}: expected at least one source and one target location, got line '{}'",
                    cli.query_in,
                    line
                );
            }

            queries.push(Query { source: source.unwrap(), targets, distances: Vec::new() });
        }

        Ok(queries)
    })
    .with_context(|| format!("Failed to read query file: {:?}", cli.query_in))?;

    info!("Initialising Dijkstra data structures");
    let mut dijkstra = GfaDijkstra::new(&graph);

    info!("Executing queries");
    let progress_bar =
        ProgressBar::new(queries.len().try_into().unwrap()).with_message("Executing queries");
    let start_time = Instant::now();

    for query in &mut queries {
        let paths = if query.targets.len() == 1 {
            dijkstra.shortest_paths(
                query.source,
                &SingleGfaLocationIndex::new_target(query.targets[0]),
            )
        } else {
            dijkstra.shortest_paths(
                query.source,
                &MultiGfaLocationIndex::new_targets(&graph, query.targets.iter().copied()),
            )
        };
        query.distances = query
            .targets
            .iter()
            .map(|&target| paths.get(&target).map(|path| path.length()).into())
            .collect();

        progress_bar.inc(1);
    }

    let end_time = Instant::now();
    progress_bar.finish_and_clear();

    info!(
        "Finished executing {} queries in {:.2?} ({:.0}µs per query)",
        queries.len(),
        end_time - start_time,
        (end_time - start_time).as_secs_f64() / queries.len() as f64 * 1_000_000.0
    );

    info!("Writing query results to file {:?}", cli.query_out);
    write_optionally_compressed_file(&cli.query_out, |writer| {
        for query in &queries {
            write!(
                writer,
                "{}\t{}\t{}",
                graph.node_name(query.source.node().into_bidirected()),
                query.source.offset(),
                if query.source.node().is_forward() {
                    "+"
                } else {
                    "-"
                },
            )?;

            for (target, distance) in query.targets.iter().zip(&query.distances) {
                write!(
                    writer,
                    "{}\t{}\t{}\t{}",
                    graph.node_name(target.node().into_bidirected()),
                    target.offset(),
                    if target.node().is_forward() { "+" } else { "-" },
                    distance
                        .into_option()
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "None".to_string()),
                )?;
            }

            writeln!(writer)?;
        }
        Ok(())
    })
    .with_context(|| format!("Failed to write query results to file: {:?}", cli.query_out))?;

    Ok(())
}
