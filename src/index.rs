use std::{fs::File, path::PathBuf};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray, index::GraphIndexInteger, io::gfa1::read_gfa1,
};
use clap::Parser;
use log::{LevelFilter, info};
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

    /// The output file for the index.
    #[clap(long)]
    index_out: PathBuf,

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

    info!("Building overlay");
    let overlay = SPQRDecompositionOverlay::new(&graph, &spqr_decomposition);

    info!("Writing index to file {:?}", cli.index_out);
    {
        let mut file = File::create(&cli.index_out)
            .with_context(|| format!("Failed to create index output file {:?}", cli.index_out))?;
        overlay
            .write_binary(&mut file)
            .with_context(|| format!("Failed to write index to file {:?}", cli.index_out))?;
    }

    info!("Indexing completed successfully");
    Ok(())
}
