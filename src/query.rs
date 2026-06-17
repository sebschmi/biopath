use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    str::FromStr,
    time::Instant,
};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::GraphIndexInteger,
    io::gfa1::{PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use indicatif::ProgressBar;
use log::{LevelFilter, info, warn};
use serde::{Deserialize, Serialize};
use spqr_shortest_path_index::{
    dijkstra::GfaDijkstra,
    location_index::{multi::MultiGfaLocationIndex, single::SingleGfaLocationIndex},
    spqr_decomposition_overlay::{SPQRDecompositionOverlay, dijkstra::OverlayDijkstra},
};
use spqr_tree::{decomposition::SPQRDecomposition, graph::StaticGraph};

use crate::{
    io_util::{
        open_optionally_compressed_file, read_optionally_compressed_file,
        write_human_readable_time_to_string, write_optionally_compressed_file,
    },
    query_file::parse_query_file,
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

    /// If specified, write timing information to the given file in JSON format.
    #[clap(long)]
    timing_out: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct QueryTiming {
    graph_reading_time: f64,
    node_name_index_building_time: f64,
    spqr_reading_time: f64,
    index_reading_time: f64,
    query_reading_time: f64,
    dijkstra_initialisation_time: f64,
    single_query_execution_times: Vec<f64>,
    total_query_execution_time: f64,
    query_writing_time: f64,

    compute_time: f64,
    io_time: f64,

    total_time: f64,
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
    let total_start_timestamp = Instant::now();

    let start_timestamp = Instant::now();
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
    let graph_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Building node name index");
    let node_name_index: HashMap<_, _> = graph
        .node_indices()
        .map(|node_index| (graph.node_name(node_index), node_index))
        .collect();
    let node_name_index_building_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Reading SPQR decomposition from file {:?}", cli.spqr_in);
    let spqr_decomposition =
        read_optionally_compressed_file(cli.spqr_in.as_ref().unwrap(), |reader| {
            SPQRDecomposition::read_plain_spqr(&graph, reader).with_context(|| {
                format!("Failed to parse SPQR decomposition file {:?}", cli.spqr_in)
            })
        })
        .with_context(|| format!("Failed to read SPQR file: {:?}", cli.spqr_in))?;
    let spqr_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Reading index from file {:?}", cli.index_in);
    let overlay =
        SPQRDecompositionOverlay::read_binary(&graph, &spqr_decomposition, index_file_reader)
            .with_context(|| format!("Failed to read index file: {:?}", cli.index_in))?;
    let index_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Reading queries from file {:?}", cli.query_in);
    let mut queries = read_optionally_compressed_file(&cli.query_in, |reader| {
        parse_query_file(reader, &cli.query_in, |node_name| {
            node_name_index.get(node_name).copied()
        })
    })
    .with_context(|| format!("Failed to read query file: {:?}", cli.query_in))?;
    let query_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Initialising overlay Dijkstra data structures");
    let mut dijkstra = OverlayDijkstra::new(&overlay);
    let dijkstra_initialisation_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    let mut single_query_execution_times = Vec::with_capacity(queries.len());
    info!("Executing queries");
    let progress_bar =
        ProgressBar::new(queries.len().try_into().unwrap()).with_message("Executing queries");

    for query in &mut queries {
        let single_query_start_timestamp = Instant::now();
        let paths = if query.targets().len() == 1 {
            dijkstra.shortest_paths(
                *query.source(),
                &SingleGfaLocationIndex::new_target(query.targets()[0]),
            )
        } else {
            dijkstra.shortest_paths(
                *query.source(),
                &MultiGfaLocationIndex::new_targets(&graph, query.targets().iter().copied()),
            )
        };
        single_query_execution_times.push(single_query_start_timestamp.elapsed());

        query.set_distances(
            query
                .targets()
                .iter()
                .map(|&target| paths.get(&target).map(|path| path.length()).into())
                .collect(),
        );

        progress_bar.inc(1);
    }

    progress_bar.finish_and_clear();
    let total_query_execution_time = start_timestamp.elapsed();

    info!(
        "Finished executing {} queries in {:.2?} ({:.0}µs per query)",
        queries.len(),
        total_query_execution_time,
        total_query_execution_time.as_secs_f64() / queries.len() as f64 * 1_000_000.0,
    );

    let start_timestamp = Instant::now();
    info!("Writing query results to file {:?}", cli.query_out);
    write_optionally_compressed_file(&cli.query_out, |writer| {
        for query in &queries {
            write!(
                writer,
                "{}\t{}\t{}",
                graph.node_name(query.source().node().into_bidirected()),
                query.source().offset(),
                if query.source().node().is_forward() {
                    "+"
                } else {
                    "-"
                },
            )?;

            for (target, distance) in query.targets().iter().zip(query.distances()) {
                write!(
                    writer,
                    "\t{}\t{}\t{}\t{}",
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
    let query_writing_time = start_timestamp.elapsed();

    let total_time = total_start_timestamp.elapsed();
    info!("Querying completed successfully");

    let compute_time = dijkstra_initialisation_time + total_query_execution_time;
    let io_time = graph_reading_time
        + node_name_index_building_time
        + spqr_reading_time
        + index_reading_time
        + query_reading_time
        + query_writing_time;

    let timing_info = QueryTiming {
        graph_reading_time: graph_reading_time.as_secs_f64(),
        node_name_index_building_time: node_name_index_building_time.as_secs_f64(),
        spqr_reading_time: spqr_reading_time.as_secs_f64(),
        index_reading_time: index_reading_time.as_secs_f64(),
        query_reading_time: query_reading_time.as_secs_f64(),
        dijkstra_initialisation_time: dijkstra_initialisation_time.as_secs_f64(),
        single_query_execution_times: single_query_execution_times
            .iter()
            .map(|t| t.as_secs_f64())
            .collect(),
        total_query_execution_time: total_query_execution_time.as_secs_f64(),
        query_writing_time: query_writing_time.as_secs_f64(),

        compute_time: compute_time.as_secs_f64(),
        io_time: io_time.as_secs_f64(),

        total_time: total_time.as_secs_f64(),
    };

    info!(
        "Query timings in seconds:\n{}",
        timing_info.write_human_readable_to_string(),
    );

    if let Some(timing_out) = cli.timing_out {
        info!("Writing timing information to file {timing_out:?}");

        let timing_out_file = File::create(&timing_out)
            .with_context(|| format!("Failed to create timing output file: {timing_out:?}"))?;

        serde_json::to_writer_pretty(timing_out_file, &timing_info).with_context(|| {
            format!("Failed to write timing information to file: {timing_out:?}")
        })?;

        info!("Timing information written successfully");
    }

    Ok(())
}

fn run_without_index<IndexType: GraphIndexInteger + FromStr>(cli: Cli) -> anyhow::Result<()>
where
    <IndexType as FromStr>::Err: std::error::Error + Send + Sync + 'static,
{
    let total_start_timestamp = Instant::now();

    let start_timestamp = Instant::now();
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
    let graph_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Building node name index");
    let node_name_index: HashMap<_, _> = graph
        .node_indices()
        .map(|node_index| (graph.node_name(node_index), node_index))
        .collect();
    let node_name_index_building_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Reading queries from file {:?}", cli.query_in);
    let mut queries = read_optionally_compressed_file(&cli.query_in, |reader| {
        parse_query_file(reader, &cli.query_in, |node_name| {
            node_name_index.get(node_name).copied()
        })
    })
    .with_context(|| format!("Failed to read query file: {:?}", cli.query_in))?;
    let query_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Initialising Dijkstra data structures");
    let mut dijkstra = GfaDijkstra::new(&graph);
    let dijkstra_initialisation_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    let mut single_query_execution_times = Vec::with_capacity(queries.len());
    info!("Executing queries");
    let progress_bar =
        ProgressBar::new(queries.len().try_into().unwrap()).with_message("Executing queries");

    for query in &mut queries {
        let single_query_start_timestamp = Instant::now();
        let paths = if query.targets().len() == 1 {
            dijkstra.shortest_paths(
                *query.source(),
                &SingleGfaLocationIndex::new_target(*query.targets().first().unwrap()),
            )
        } else {
            dijkstra.shortest_paths(
                *query.source(),
                &MultiGfaLocationIndex::new_targets(&graph, query.targets().iter().copied()),
            )
        };
        single_query_execution_times.push(single_query_start_timestamp.elapsed());

        query.set_distances(
            query
                .targets()
                .iter()
                .map(|&target| paths.get(&target).map(|path| path.length()).into())
                .collect(),
        );

        progress_bar.inc(1);
    }

    progress_bar.finish_and_clear();
    let total_query_execution_time = start_timestamp.elapsed();

    info!(
        "Finished executing {} queries in {:.2?} ({:.0}µs per query)",
        queries.len(),
        total_query_execution_time,
        total_query_execution_time.as_secs_f64() / queries.len() as f64 * 1_000_000.0,
    );

    let start_timestamp = Instant::now();
    info!("Writing query results to file {:?}", cli.query_out);
    write_optionally_compressed_file(&cli.query_out, |writer| {
        for query in &queries {
            write!(
                writer,
                "{}\t{}\t{}",
                graph.node_name(query.source().node().into_bidirected()),
                query.source().offset(),
                if query.source().node().is_forward() {
                    "+"
                } else {
                    "-"
                },
            )?;

            for (target, distance) in query.targets().iter().zip(query.distances()) {
                write!(
                    writer,
                    "\t{}\t{}\t{}\t{}",
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
    let query_writing_time = start_timestamp.elapsed();

    let total_time = total_start_timestamp.elapsed();
    info!("Querying completed successfully");

    let compute_time = dijkstra_initialisation_time + total_query_execution_time;
    let io_time = graph_reading_time
        + node_name_index_building_time
        + query_reading_time
        + query_writing_time;

    let timing_info = QueryTiming {
        graph_reading_time: graph_reading_time.as_secs_f64(),
        node_name_index_building_time: node_name_index_building_time.as_secs_f64(),
        spqr_reading_time: Default::default(),
        index_reading_time: Default::default(),
        query_reading_time: query_reading_time.as_secs_f64(),
        dijkstra_initialisation_time: dijkstra_initialisation_time.as_secs_f64(),
        single_query_execution_times: single_query_execution_times
            .iter()
            .map(|t| t.as_secs_f64())
            .collect(),
        total_query_execution_time: total_query_execution_time.as_secs_f64(),
        query_writing_time: query_writing_time.as_secs_f64(),

        compute_time: compute_time.as_secs_f64(),
        io_time: io_time.as_secs_f64(),

        total_time: total_time.as_secs_f64(),
    };

    info!(
        "Query timings in seconds:\n{}",
        timing_info.write_human_readable_to_string(),
    );

    if let Some(timing_out) = cli.timing_out {
        info!("Writing timing information to file {timing_out:?}");

        let timing_out_file = File::create(&timing_out)
            .with_context(|| format!("Failed to create timing output file: {timing_out:?}"))?;

        serde_json::to_writer_pretty(timing_out_file, &timing_info).with_context(|| {
            format!("Failed to write timing information to file: {timing_out:?}")
        })?;

        info!("Timing information written successfully");
    }

    Ok(())
}

impl QueryTiming {
    fn write_human_readable(&self, mut writer: impl Write) -> std::io::Result<()> {
        let Self {
            graph_reading_time,
            node_name_index_building_time,
            spqr_reading_time,
            index_reading_time,
            query_reading_time,
            dijkstra_initialisation_time,
            single_query_execution_times,
            total_query_execution_time,
            query_writing_time,

            compute_time,
            io_time,

            total_time,
        } = self;

        writeln!(
            writer,
            "graph_reading_time: {}",
            write_human_readable_time_to_string(*graph_reading_time),
        )?;
        writeln!(
            writer,
            "node_name_index_building_time: {}",
            write_human_readable_time_to_string(*node_name_index_building_time),
        )?;
        writeln!(
            writer,
            "spqr_reading_time: {}",
            write_human_readable_time_to_string(*spqr_reading_time),
        )?;
        writeln!(
            writer,
            "index_reading_time: {}",
            write_human_readable_time_to_string(*index_reading_time),
        )?;
        writeln!(
            writer,
            "query_reading_time: {}",
            write_human_readable_time_to_string(*query_reading_time),
        )?;
        writeln!(
            writer,
            "dijkstra_initialisation_time: {}",
            write_human_readable_time_to_string(*dijkstra_initialisation_time),
        )?;
        writeln!(
            writer,
            "total_query_execution_time: {}",
            write_human_readable_time_to_string(*total_query_execution_time),
        )?;
        writeln!(
            writer,
            "average_query_execution_time: {}",
            write_human_readable_time_to_string(
                single_query_execution_times.iter().sum::<f64>()
                    / single_query_execution_times.len() as f64
            ),
        )?;
        writeln!(
            writer,
            "query_count: {}",
            single_query_execution_times.len(),
        )?;
        writeln!(
            writer,
            "query_writing_time: {}",
            write_human_readable_time_to_string(*query_writing_time),
        )?;

        writeln!(
            writer,
            "compute_time: {}",
            write_human_readable_time_to_string(*compute_time),
        )?;
        writeln!(
            writer,
            "io_time: {}",
            write_human_readable_time_to_string(*io_time),
        )?;

        writeln!(
            writer,
            "total_time: {}",
            write_human_readable_time_to_string(*total_time),
        )?;

        Ok(())
    }

    fn write_human_readable_to_string(&self) -> String {
        let mut buf = Vec::new();
        self.write_human_readable(&mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }
}
