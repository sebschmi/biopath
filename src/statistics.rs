use std::{fs::File, io::Write, path::PathBuf};

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::GraphIndexInteger,
    io::gfa1::{PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use itertools::Itertools;
use log::{LevelFilter, info};
use serde::{Deserialize, Serialize};
use spqr_tree::decomposition::SPQRDecomposition;

use crate::io_util::read_optionally_compressed_file;

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

    /// The output file for the statistics in JSON format.
    #[clap(long)]
    statistics_json_out: Option<PathBuf>,

    /// The output file for the statistics in TOML format.
    #[clap(long)]
    statistics_toml_out: Option<PathBuf>,

    /// The integer size to use in all data structures.
    /// Supported values are 8, 16, 32, and 64.
    /// If the program crashes during reading the graph, try using a larger word size.
    #[clap(long, default_value = "32")]
    word_size: u8,
}

#[derive(Debug, Serialize, Deserialize)]
struct Statistics<IndexType> {
    node_count: IndexType,
    edge_count: IndexType,

    component_count: IndexType,
    block_count: IndexType,
    spqr_node_count: IndexType,

    component_node_counts: Vec<IndexType>,
    block_node_counts: Vec<IndexType>,
    spqr_node_node_counts: Vec<IndexType>,

    component_block_counts: Vec<IndexType>,
    block_spqr_node_counts: Vec<IndexType>,
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

    info!("Reading SPQR decomposition from file {:?}", cli.spqr_in);
    let spqr_decomposition = read_optionally_compressed_file(&cli.spqr_in, |reader| {
        SPQRDecomposition::read_plain_spqr(&graph, reader)
            .with_context(|| format!("Failed to parse SPQR decomposition file {:?}", cli.spqr_in))
    })
    .with_context(|| format!("Failed to read SPQR file: {:?}", cli.spqr_in))?;

    info!("Collecting statistics");
    let statistics = Statistics {
        node_count: graph.node_count(),
        edge_count: graph.edge_count(),

        component_count: spqr_decomposition.component_count(),
        block_count: spqr_decomposition.block_count(),
        spqr_node_count: spqr_decomposition.spqr_node_count(),

        component_node_counts: spqr_decomposition
            .iter_components()
            .map(|component| component.1.node_count())
            .sorted()
            .rev()
            .collect(),
        block_node_counts: spqr_decomposition
            .iter_blocks()
            .map(|block| block.1.node_count())
            .sorted()
            .rev()
            .collect(),
        spqr_node_node_counts: spqr_decomposition
            .iter_spqr_nodes()
            .map(|spqr_node| spqr_node.1.node_count())
            .sorted()
            .rev()
            .collect(),

        component_block_counts: spqr_decomposition
            .iter_components()
            .map(|component| component.1.block_count())
            .sorted()
            .rev()
            .collect(),
        block_spqr_node_counts: spqr_decomposition
            .iter_blocks()
            .map(|block| block.1.spqr_node_count())
            .sorted()
            .rev()
            .collect(),
    };

    info!("Printing short statistics");
    println!("node_count = {}", statistics.node_count);
    println!("edge_count = {}", statistics.edge_count);
    println!("component_count = {}", statistics.component_count);
    println!("block_count = {}", statistics.block_count);
    println!("spqr_node_count = {}", statistics.spqr_node_count);

    if let Some(json_out) = cli.statistics_json_out {
        info!("Writing full statistics to JSON file {:?}", json_out);
        let mut file = File::create(&json_out)
            .with_context(|| format!("Failed to create JSON output file {:?}", json_out))?;
        serde_json::to_writer(&mut file, &statistics)
            .with_context(|| format!("Failed to write statistics to JSON file {:?}", json_out))?;
    }

    if let Some(toml_out) = cli.statistics_toml_out {
        info!("Writing full statistics to TOML file {:?}", toml_out);
        let mut file = File::create(&toml_out)
            .with_context(|| format!("Failed to create TOML output file {:?}", toml_out))?;
        let toml_string =
            toml::to_string(&statistics).with_context(|| "Failed to format statistics as TOML")?;
        file.write_all(toml_string.as_bytes())
            .with_context(|| format!("Failed to write TOML output file {:?}", toml_out))?;
    }

    info!("Finished");
    Ok(())
}
