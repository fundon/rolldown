use oxc::span::Atom;
use rolldown_common::{EntryPoint, EntryPointKind, ModuleId, NamedImport, Specifier, SymbolRef};
use rustc_hash::FxHashMap;
use string_wizard::{Joiner, JoinerOptions};

use crate::{
  bundler::{
    bundle::output::RenderedModule, chunk_graph::ChunkGraph, module::ModuleRenderContext,
    options::output_options::OutputOptions, stages::link_stage::LinkStageOutput,
    utils::bitset::BitSet,
  },
  error::BatchedResult,
  FileNameTemplate, InputOptions,
};

use super::ChunkId;

#[derive(Debug)]
pub struct CrossChunkImportItem {
  pub export_alias: Option<Specifier>,
  pub import_ref: SymbolRef,
}

#[derive(Debug, Default)]
pub struct Chunk {
  pub entry_point: Option<EntryPoint>,
  pub modules: Vec<ModuleId>,
  pub name: Option<String>,
  pub file_name: Option<String>,
  pub canonical_names: FxHashMap<SymbolRef, Atom>,
  pub bits: BitSet,
  pub imports_from_other_chunks: FxHashMap<ChunkId, Vec<CrossChunkImportItem>>,
  pub imports_from_external_modules: FxHashMap<ModuleId, Vec<NamedImport>>,
  // meaningless if the chunk is an entrypoint
  pub exports_to_other_chunks: FxHashMap<SymbolRef, Atom>,
}

impl Chunk {
  pub fn new(
    name: Option<String>,
    entry_point: Option<EntryPoint>,
    bits: BitSet,
    modules: Vec<ModuleId>,
  ) -> Self {
    Self { entry_point, modules, name, bits, ..Self::default() }
  }

  pub fn file_name_template<'a>(
    &mut self,
    output_options: &'a OutputOptions,
  ) -> &'a FileNameTemplate {
    if matches!(self.entry_point, Some(EntryPoint { kind: EntryPointKind::UserDefined, .. })) {
      &output_options.entry_file_names
    } else {
      &output_options.chunk_file_names
    }
  }

  #[allow(clippy::unnecessary_wraps, clippy::cast_possible_truncation)]
  pub fn render(
    &self,
    input_options: &InputOptions,
    graph: &LinkStageOutput,
    chunk_graph: &ChunkGraph,
    output_options: &OutputOptions,
  ) -> BatchedResult<(String, FxHashMap<String, RenderedModule>)> {
    use rayon::prelude::*;
    let mut rendered_modules = FxHashMap::default();
    let mut joiner = Joiner::with_options(JoinerOptions { separator: Some("\n".to_string()) });
    joiner.append(self.render_imports_for_esm(graph, chunk_graph));
    self
      .modules
      .par_iter()
      .copied()
      .map(|id| &graph.modules[id])
      .filter_map(|m| match m {
        crate::bundler::module::Module::Normal(m) => {
          let rendered_content = m.render(
            &ModuleRenderContext {
              canonical_names: &self.canonical_names,
              graph,
              chunk_graph,
              input_options,
            },
            &graph.ast_table[m.id],
          );
          Some((
            m.resource_id.expect_file().to_string(),
            RenderedModule {
              original_length: m.source.len().try_into().unwrap(),
              rendered_length: rendered_content
                .as_ref()
                .map(|c| c.len() as u32)
                .unwrap_or_default(),
            },
            rendered_content,
          ))
        }
        crate::bundler::module::Module::External(_) => None,
      })
      .collect::<Vec<_>>()
      .into_iter()
      .for_each(|(module_path, rendered_module, rendered_content)| {
        if let Some(rendered_content) = rendered_content {
          joiner.append(rendered_content);
        }
        rendered_modules.insert(module_path, rendered_module);
      });

    if let Some(exports) = self.render_exports(graph, output_options) {
      joiner.append(exports);
    }

    Ok((joiner.join(), rendered_modules))
  }
}
