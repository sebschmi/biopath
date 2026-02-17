use std::{fs::File, io::Read, path::PathBuf};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray, index::GraphIndexInteger, io::gfa1::read_gfa1,
};
use clap::Parser;
use log::{LevelFilter, error, info};
use spqr_shortest_path_index::spqr_decomposition_overlay::SPQRDecompositionOverlay;
use spqr_tree::io::plain_spqr_file::read_plain_spqr;

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

    /// The index file.
    #[clap(long)]
    index_in: PathBuf,
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    // Read word size from index file first.
    let mut index_file = File::open(&cli.index_in)
        .with_context(|| format!("Failed to open index file {:?}", cli.index_in))?;
    let mut word_size_bytes = [0u8; 1];
    index_file
        .read_exact(&mut word_size_bytes)
        .with_context(|| format!("Failed to read index header from file {:?}", cli.index_in))?;
    let word_size = word_size_bytes[0];

    info!(
        "Discovered word size {} bits from index file header",
        word_size
    );

    match word_size {
        8 => run_with_word_size::<u8>(cli, index_file),
        16 => run_with_word_size::<u16>(cli, index_file),
        32 => run_with_word_size::<u32>(cli, index_file),
        64 => run_with_word_size::<u64>(cli, index_file),
        _ => anyhow::bail!(
            "Unsupported word size: {}. Supported are 8, 16, 32 and 64.",
            word_size
        ),
    }
}

fn run_with_word_size<IndexType: GraphIndexInteger>(
    cli: Cli,
    mut index_file: File,
) -> anyhow::Result<()> {
    info!("Reading graph from GFA file {:?}", cli.graph_gfa_in);
    let graph: BidirectedAdjacencyArray<IndexType, _, _> = {
        let mut file = File::open(&cli.graph_gfa_in)
            .with_context(|| format!("Failed to open GFA file {:?}", cli.graph_gfa_in))?;
        read_gfa1(&mut file)
            .with_context(|| format!("Failed to parse GFA file {:?}", cli.graph_gfa_in))?
    };
    info!(
        "Graph has {} nodes and {} edges",
        graph.node_count(),
        graph.edge_count(),
    );

    info!("Reading SPQR decomposition from file {:?}", cli.spqr_in);
    let spqr_decomposition = {
        let mut file = File::open(&cli.spqr_in)
            .with_context(|| format!("Failed to open SPQR decomposition file {:?}", cli.spqr_in))?;
        read_plain_spqr(&graph, &mut file)
            .with_context(|| format!("Failed to parse SPQR decomposition file {:?}", cli.spqr_in))?
    };

    info!("Reading index from file {:?}", cli.index_in);
    let _overlay =
        SPQRDecompositionOverlay::read_binary(&graph, &spqr_decomposition, &mut index_file)
            .with_context(|| format!("Failed to parse index file {:?}", cli.index_in))?;
    drop(index_file);

    error!("Query not yet implemented");
    Ok(())
}
