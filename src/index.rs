use std::{fs::File, io::Write, path::PathBuf, time::Instant};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::GraphIndexInteger,
    io::gfa1::{PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use log::{LevelFilter, info};
use serde::{Deserialize, Serialize};
use spqr_shortest_path_index::spqr_decomposition_overlay::SPQRDecompositionOverlay;
use spqr_tree::decomposition::SPQRDecomposition;

use crate::io_util::{
    read_optionally_compressed_file, write_human_readable_time_to_string,
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
    #[clap(long)]
    spqr_in: PathBuf,

    /// The output file for the index.
    #[clap(long)]
    index_out: PathBuf,

    /// If specified, write timing information to the given file in JSON format.
    #[clap(long)]
    timing_out: Option<PathBuf>,

    /// The integer size to use in all data structures.
    /// Supported values are 8, 16, 32, and 64.
    /// If the program crashes during reading the graph, try using a larger word size.
    #[clap(long, default_value = "32")]
    word_size: u8,
}

#[derive(Serialize, Deserialize)]
struct IndexTiming {
    graph_reading_time: f64,
    spqr_reading_time: f64,
    overlay_building_time: f64,
    index_writing_time: f64,

    compute_time: f64,
    io_time: f64,

    total_time: f64,
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.word_size {
        8 => run_with_word_size::<u8>(cli),
        16 => run_with_word_size::<u16>(cli),
        32 => run_with_word_size::<u32>(cli),
        64 => run_with_word_size::<u64>(cli),
        _ => anyhow::bail!(
            "Unsupported word size: {}. Supported are 8, 16, 32 and 64.",
            cli.word_size
        ),
    }
}

fn run_with_word_size<IndexType: GraphIndexInteger>(cli: Cli) -> anyhow::Result<()> {
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
    info!("Reading SPQR decomposition from file {:?}", cli.spqr_in);
    let spqr_decomposition = read_optionally_compressed_file(&cli.spqr_in, |reader| {
        SPQRDecomposition::read_plain_spqr(&graph, reader)
            .with_context(|| format!("Failed to parse SPQR decomposition file {:?}", cli.spqr_in))
    })
    .with_context(|| format!("Failed to read SPQR file: {:?}", cli.spqr_in))?;
    let spqr_reading_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Building overlay");
    let overlay = SPQRDecompositionOverlay::new(&graph, &spqr_decomposition);
    let overlay_building_time = start_timestamp.elapsed();

    let start_timestamp = Instant::now();
    info!("Writing index to file {:?}", cli.index_out);
    write_optionally_compressed_file(&cli.index_out, |writer| {
        writer
            .write_all(&[u8::try_from(std::mem::size_of::<IndexType>() * 8).unwrap()])
            .with_context(|| format!("Failed to write index header to file {:?}", cli.index_out))?;
        overlay
            .write_binary(writer)
            .with_context(|| format!("I/O error while writing index to file: {:?}", cli.index_out))
    })
    .with_context(|| format!("Failed to write index to file: {:?}", cli.index_out))?;
    let index_writing_time = start_timestamp.elapsed();

    let total_time = total_start_timestamp.elapsed();
    info!("Indexing completed successfully");

    let compute_time = overlay_building_time;
    let io_time = graph_reading_time + spqr_reading_time + index_writing_time;

    let timing_info = IndexTiming {
        graph_reading_time: graph_reading_time.as_secs_f64(),
        spqr_reading_time: spqr_reading_time.as_secs_f64(),
        overlay_building_time: overlay_building_time.as_secs_f64(),
        index_writing_time: index_writing_time.as_secs_f64(),

        compute_time: compute_time.as_secs_f64(),
        io_time: io_time.as_secs_f64(),

        total_time: total_time.as_secs_f64(),
    };

    info!(
        "Index timings in seconds:\n{}",
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

impl IndexTiming {
    fn write_human_readable(&self, mut writer: impl Write) -> std::io::Result<()> {
        let Self {
            graph_reading_time,
            spqr_reading_time,
            overlay_building_time,
            index_writing_time,

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
            "spqr_reading_time: {}",
            write_human_readable_time_to_string(*spqr_reading_time),
        )?;
        writeln!(
            writer,
            "overlay_building_time: {}",
            write_human_readable_time_to_string(*overlay_building_time),
        )?;
        writeln!(
            writer,
            "index_writing_time: {}",
            write_human_readable_time_to_string(*index_writing_time),
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
