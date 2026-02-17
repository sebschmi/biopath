use std::{io::Write, path::PathBuf};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::GraphIndexInteger,
    io::gfa1::{PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use log::{LevelFilter, info};
use spqr_shortest_path_index::spqr_decomposition_overlay::SPQRDecompositionOverlay;
use spqr_tree::decomposition::SPQRDecomposition;

use crate::io_util::{read_optionally_compressed_file, write_optionally_compressed_file};

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

    info!("Reading SPQR decomposition from file {:?}", cli.spqr_in);
    let spqr_decomposition = read_optionally_compressed_file(&cli.spqr_in, |reader| {
        SPQRDecomposition::read_plain_spqr(&graph, reader)
            .with_context(|| format!("Failed to parse SPQR decomposition file {:?}", cli.spqr_in))
    })
    .with_context(|| format!("Failed to read SPQR file: {:?}", cli.spqr_in))?;

    info!("Building overlay");
    let overlay = SPQRDecompositionOverlay::new(&graph, &spqr_decomposition);

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

    info!("Indexing completed successfully");
    Ok(())
}
