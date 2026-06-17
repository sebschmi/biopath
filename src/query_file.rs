use std::{
    io::{BufRead, Write},
    path::Path,
    str::FromStr,
};

use anyhow::{Context, anyhow};
use bidirected_adjacency_array::index::{DirectedNodeIndex, GraphIndexInteger, NodeIndex};
use itertools::Itertools;
use spqr_shortest_path_index::{
    location::{GfaLocation, GfaNodeOffset},
    path::OptionalGfaPathLength,
};

pub struct Query<IndexType> {
    source: GfaLocation<IndexType>,
    targets: Vec<GfaLocation<IndexType>>,
    distances: Vec<OptionalGfaPathLength<IndexType>>,
}

impl<IndexType> Query<IndexType> {
    pub fn new(source: GfaLocation<IndexType>, targets: Vec<GfaLocation<IndexType>>) -> Self {
        Self {
            source,
            targets,
            distances: Vec::new(),
        }
    }

    pub fn source(&self) -> &GfaLocation<IndexType> {
        &self.source
    }

    pub fn targets(&self) -> &[GfaLocation<IndexType>] {
        &self.targets
    }

    pub fn distances(&self) -> &[OptionalGfaPathLength<IndexType>] {
        &self.distances
    }

    pub fn set_distances(&mut self, distances: Vec<OptionalGfaPathLength<IndexType>>) {
        self.distances = distances;
    }
}

pub fn read_query_file<IndexType: GraphIndexInteger + FromStr>(
    reader: impl BufRead,
    file_name: impl AsRef<Path>,
    name_to_node_index: impl Fn(&str) -> Option<NodeIndex<IndexType>>,
) -> anyhow::Result<Vec<Query<IndexType>>>
where
    <IndexType as FromStr>::Err: std::error::Error + Send + Sync + 'static,
{
    let mut queries = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| {
            format!(
                "Failed to read line from query file: {:?}",
                file_name.as_ref(),
            )
        })?;

        let mut source = None;
        let mut targets = Vec::new();

        let columns = line.trim().split('\t').collect_vec();
        for column in columns.chunks(3) {
            if column.len() != 3 {
                anyhow::bail!(
                    "Invalid query line in file {:?}: expected a number of columns that is divisible by 3, got {:?}",
                    file_name.as_ref(),
                    columns.len(),
                );
            }

            let node_name = column[0];
            let forward = match column[1] {
                "+" => true,
                "-" => false,
                _ => anyhow::bail!(
                    "Invalid query line in file {:?}: expected orientation to be either '+' or '-', got {:?}",
                    file_name.as_ref(),
                    column[1],
                ),
            };
            let offset = column[2]
                .parse::<IndexType>()
                .map(GfaNodeOffset::from_raw)
                .with_context(|| {
                    format!(
                        "Invalid query line in file {:?}: failed to parse offset {:?}",
                        file_name.as_ref(),
                        column[2],
                    )
                })?;

            let location = GfaLocation::new(
                DirectedNodeIndex::from_bidirected(
                    name_to_node_index(node_name)
                        .ok_or_else(|| anyhow!("Node name {node_name:?} not found in index"))?,
                    forward,
                ),
                offset,
            );
            if source.is_none() {
                source = Some(location);
            } else {
                targets.push(location);
            }
        }

        if source.is_none() || targets.is_empty() {
            anyhow::bail!(
                "Invalid query line in file {:?}: expected at least one source and one target location, got line {:?}",
                file_name.as_ref(),
                line,
            );
        }

        queries.push(Query::new(source.unwrap(), targets));
    }

    Ok(queries)
}

pub fn write_query_file<IndexType: GraphIndexInteger>(
    mut writer: impl Write,
    queries: &[Query<IndexType>],
    node_to_name_index: impl Fn(NodeIndex<IndexType>) -> Option<String>,
) -> anyhow::Result<()> {
    for query in queries {
        write_location(&mut writer, &query.source, &node_to_name_index)
            .with_context(|| "Error writing query source location".to_string())?;

        for target in &query.targets {
            write!(writer, "\t").with_context(|| "Error writing query".to_string())?;
            write_location(&mut writer, target, &node_to_name_index)
                .with_context(|| "Error writing query target location".to_string())?;
        }

        writeln!(writer).with_context(|| "Error writing query".to_string())?;
    }

    Ok(())
}

fn write_location<IndexType: GraphIndexInteger>(
    mut writer: impl Write,
    location: &GfaLocation<IndexType>,
    node_to_name_index: impl Fn(NodeIndex<IndexType>) -> Option<String>,
) -> anyhow::Result<()> {
    let node = location.node().into_bidirected();
    let node_name = node_to_name_index(node)
        .ok_or_else(|| anyhow!("Node index {node:?} not found in index"))?;
    let orientation = if location.node().is_forward() {
        "+"
    } else {
        "-"
    };
    let offset = location.offset();
    write!(writer, "{}\t{}\t{}", node_name, orientation, offset).map_err(Into::into)
}
