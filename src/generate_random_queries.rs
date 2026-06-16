use std::{fs::File, io::Write, path::PathBuf, time::Instant};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::GraphIndexInteger,
    io::gfa1::{PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use log::{LevelFilter, info};

use crate::io_util::{read_optionally_compressed_file, write_optionally_compressed_file};

#[derive(Parser)]
pub struct Cli {
    #[clap(long, default_value = "info")]
    pub(crate) log_level: LevelFilter,

    /// The GFA file containing the graph to index.
    #[clap(long)]
    graph_gfa_in: PathBuf,

    /// The output file for the generated queries.
    ///
    /// The file format is tab-separated.
    /// The columns are `source_node_id`, `source_orientation`, `source_offset`, `target_node_id`, `target_orientation`, `target_offset`.
    /// The last three columns can be repeated to specify multiple target locations for the same source.
    #[clap(long)]
    query_out: PathBuf,

    /// The number of random queries to generate.
    #[clap(long)]
    amount: usize,

    /// The integer size to use in all data structures.
    /// Supported values are 8, 16, 32, and 64.
    /// If the program crashes during reading the graph, try using a larger word size.
    #[clap(long, default_value = "32")]
    word_size: u8,
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

    todo!("Create module for reading and writing query files.");

    Ok(())
}
