use std::path::PathBuf;

use anyhow::Context;
use bidirected_adjacency_array::{
    graph::BidirectedAdjacencyArray,
    index::{DirectedNodeIndex, GraphIndexInteger},
    io::gfa1::{GfaNodeData, PlainGfaEdgeData, PlainGfaNodeData},
};
use clap::Parser;
use indicatif::ProgressBar;
use log::{LevelFilter, info};
use rand::{Rng, RngExt, SeedableRng, rngs::SmallRng, seq::IteratorRandom};
use spqr_shortest_path_index::location::{GfaLocation, GfaNodeOffset};
use spqr_tree::graph::StaticGraph;

use crate::{
    io_util::{read_optionally_compressed_file, write_optionally_compressed_file},
    query_file::{Query, write_query_file},
};

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
    #[clap(long, default_value = "1000")]
    amount: usize,

    /// The seed for the random generator.
    #[clap(long, default_value = "0")]
    random_seed: u64,

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

    let mut rng = SmallRng::seed_from_u64(cli.random_seed);
    let mut queries = Vec::new();
    let progress_bar =
        ProgressBar::new(cli.amount.try_into().unwrap()).with_message("Generating queries");

    for _ in 0..cli.amount {
        let source_location = generate_random_location(&graph, &mut rng)?;
        let target_location = generate_random_location(&graph, &mut rng)?;
        let query = Query::new(source_location, vec![target_location]);
        queries.push(query);
        progress_bar.inc(1);
    }

    progress_bar.finish_and_clear();
    info!("Generated {} random queries", queries.len());

    info!("Writing queries to file {:?}", cli.query_out);
    write_optionally_compressed_file(&cli.query_out, |writer| {
        write_query_file(writer, &queries, |node_index| {
            Some(graph.node_name(node_index))
        })
    })
    .with_context(|| format!("Failed to write query file: {:?}", cli.query_out))?;

    info!("Finished writing query file");

    Ok(())
}

fn generate_random_location<IndexType: GraphIndexInteger, NodeData: GfaNodeData, EdgeData>(
    graph: &BidirectedAdjacencyArray<IndexType, NodeData, EdgeData>,
    rng: &mut impl Rng,
) -> anyhow::Result<GfaLocation<IndexType>> {
    let bidirected_node = graph.node_indices().choose(rng).unwrap();
    let directed_node = DirectedNodeIndex::from_bidirected(bidirected_node, rng.random_bool(0.5));
    let location = GfaLocation::new(
        directed_node,
        GfaNodeOffset::from_usize(
            (0..graph.node_data(bidirected_node).sequence().len())
                .choose(rng)
                .unwrap(),
        ),
    );

    Ok(location)
}
