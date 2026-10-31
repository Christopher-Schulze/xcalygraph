//! `derived` file: fixed-size little-endian records read zero-copy from the memory map
//! (`docs/spec.md` section 9). Strings are ids into one interned string table.

use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;

use super::codec::{Decoder, Encoder};
use super::container::{self, Container, Kind};
use crate::domain::facts::SymKind;
use crate::domain::filetype::FileType;
use crate::domain::graph::{Confidence, EdgeKind, Graph};
use crate::domain::hash::FastMap;
use crate::domain::role::Role;
use crate::domain::search::{self, FIELDS};

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct SymbolRecord {
    pub name: u32,
    /// Qualified name without the trailing name (`pkg/store.Store.`), shared by all symbols of a
    /// module and parent chain; the whole qualified name when `whole` is set.
    pub prefix: u32,
    pub signature: u32,
    pub doc: u32,
    pub file: u32,
    pub parent: u32,
    pub line: u32,
    pub end_line: u32,
    pub rank: f32,
    pub kind: u8,
    pub flags: u8,
    pub exported: u8,
    pub whole: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct FileRecord {
    pub path: u32,
    pub module: u32,
    pub lines: u32,
    pub first_symbol: u32,
    pub symbol_count: u32,
    pub ambiguous: u32,
    pub doc: u32,
    pub role: u8,
    pub file_type: u8,
    pub entry: u8,
    pub pad: u8,
}

/// One adjacency entry: `node` is the target in the forward list, the source in the reverse.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EdgeRecord {
    pub node: u32,
    pub line: u32,
    pub kind: u8,
    pub confidence: u8,
    pub pad: [u8; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModuleRecord {
    pub name: u32,
    pub files: u32,
    pub symbols: u32,
    pub layer: u32,
    pub cycle: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ModuleEdgeRecord {
    pub source: u32,
    pub target: u32,
    pub count: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct ExternalRecord {
    pub module: u32,
    pub name: u32,
    pub uses: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct EnvRecord {
    pub name: u32,
    pub node: u32,
    pub line: u32,
    pub source: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct FieldsRecord {
    pub symbol: u32,
    /// Comma-joined field names.
    pub names: u32,
}

/// Token of the search index: byte offset of its postings and their count; sorted by token text.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct TokenRecord {
    pub token: u32,
    pub start: u32,
    pub count: u32,
}

/// Term frequency of one token per field of one node, 4 bits per field (capped at 15);
/// posting lists are sorted by node and stored as varint pairs (node delta, packed).
#[derive(Debug, Clone, Copy)]
pub struct PostingRecord {
    pub node: u32,
    pub packed: u32,
}

impl PostingRecord {
    pub fn tf(&self, field: usize) -> u8 {
        ((self.packed >> (field * 4)) & 0xf) as u8
    }

    fn add(&mut self, field: usize) {
        if self.tf(field) < 15 {
            self.packed += 1 << (field * 4);
        }
    }
}

/// Token count per field of one node (BM25F length normalization), capped at 255.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct LengthRecord {
    pub lengths: [u8; FIELDS],
}

mod section {
    pub const STRINGS: usize = 0;
    pub const STRING_OFFSETS: usize = 1;
    pub const FILES: usize = 2;
    pub const SYMBOLS: usize = 3;
    pub const FORWARD_OFFSETS: usize = 4;
    pub const FORWARD: usize = 5;
    pub const REVERSE_OFFSETS: usize = 6;
    pub const REVERSE: usize = 7;
    pub const MODULES: usize = 8;
    pub const MODULE_EDGES: usize = 9;
    pub const EXTERNALS: usize = 10;
    pub const ENVS: usize = 11;
    pub const FIELDS: usize = 12;
    pub const TOKENS: usize = 13;
    pub const POSTINGS: usize = 14;
    pub const LENGTHS: usize = 15;
    pub const AVERAGES: usize = 16;
    pub const MEMBERS: usize = 17;
    pub const STALE: usize = 18;
    pub const AMBIGUOUS: usize = 19;
    pub const COUNT: usize = 20;
}

#[derive(Default)]
struct Interner {
    blob: Vec<u8>,
    offsets: Vec<u32>,
    ids: FastMap<String, u32>,
}

impl Interner {
    /// `capacity`: expected number of interned texts.
    fn new(capacity: usize) -> Self {
        let mut interner = Self {
            offsets: vec![0],
            ids: FastMap::with_capacity_and_hasher(capacity, Default::default()),
            ..Self::default()
        };
        interner.id("");
        interner
    }

    /// Appends without deduplication: for texts that are almost always unique (signatures,
    /// docs), hashing costs more than the duplicates.
    fn append(&mut self, text: &str) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let id = (self.offsets.len() - 1) as u32;
        self.blob.extend_from_slice(text.as_bytes());
        self.offsets.push(self.blob.len() as u32);
        id
    }

    fn id(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = (self.offsets.len() - 1) as u32;
        self.blob.extend_from_slice(text.as_bytes());
        self.offsets.push(self.blob.len() as u32);
        self.ids.insert(text.to_owned(), id);
        id
    }
}

/// Serializes the graph and its search index.
pub fn build(graph: &Graph) -> Vec<u8> {
    let mut strings = Interner::new(graph.files.len() + graph.symbols.len() * 2);
    let files: Vec<FileRecord> = graph
        .files
        .iter()
        .map(|file| FileRecord {
            path: strings.id(&file.path),
            module: file.module,
            lines: file.lines,
            first_symbol: file.first_symbol,
            symbol_count: file.symbol_count,
            ambiguous: file.ambiguous,
            doc: strings.append(&file.doc),
            role: file.role as u8,
            file_type: file.file_type.map_or(0, |file_type| file_type as u8),
            entry: u8::from(file.entry),
            pad: 0,
        })
        .collect();
    let symbols: Vec<SymbolRecord> = graph
        .symbols
        .iter()
        .map(|symbol| {
            let prefix = symbol.qualified.strip_suffix(symbol.name.as_str());
            SymbolRecord {
                name: strings.id(&symbol.name),
                prefix: strings.id(prefix.unwrap_or(&symbol.qualified)),
                signature: strings.append(&symbol.signature),
                doc: strings.append(&symbol.doc),
                file: symbol.file,
                parent: symbol.parent,
                line: symbol.line,
                end_line: symbol.end_line,
                rank: symbol.rank,
                kind: symbol.kind as u8,
                flags: symbol.flags,
                exported: u8::from(symbol.exported),
                whole: u8::from(prefix.is_none()),
            }
        })
        .collect();
    let nodes = graph.node_count();
    let (forward_offsets, forward) = adjacency(nodes, graph, false);
    let (reverse_offsets, reverse) = adjacency(nodes, graph, true);
    let modules: Vec<ModuleRecord> = graph
        .modules
        .iter()
        .map(|module| ModuleRecord {
            name: strings.id(&module.name),
            files: module.files,
            symbols: module.symbols,
            layer: module.layer,
            cycle: module.cycle,
        })
        .collect();
    let module_edges: Vec<ModuleEdgeRecord> = graph
        .module_edges
        .iter()
        .map(|edge| ModuleEdgeRecord {
            source: edge.source,
            target: edge.target,
            count: edge.count,
        })
        .collect();
    let externals: Vec<ExternalRecord> = graph
        .externals
        .iter()
        .map(|external| ExternalRecord {
            module: external.module,
            name: strings.id(&external.name),
            uses: external.uses,
        })
        .collect();
    let envs: Vec<EnvRecord> = graph
        .envs
        .iter()
        .map(|env| EnvRecord {
            name: strings.id(&env.name),
            node: env.node,
            line: env.line,
            source: u32::from(env.source),
        })
        .collect();
    let fields: Vec<FieldsRecord> = graph
        .fields
        .iter()
        .map(|fields| FieldsRecord {
            symbol: fields.symbol,
            names: strings.id(&fields.names.join(",")),
        })
        .collect();
    let index = search_index(graph);
    let tokens: Vec<TokenRecord> = index
        .tokens
        .iter()
        .map(|(token, start, count)| TokenRecord {
            token: strings.id(token),
            start: *start,
            count: *count,
        })
        .collect();
    let mut sections: Vec<Vec<u8>> = vec![Vec::new(); section::COUNT];
    sections[section::FILES] = bytemuck::cast_slice(&files).to_vec();
    sections[section::SYMBOLS] = bytemuck::cast_slice(&symbols).to_vec();
    sections[section::FORWARD_OFFSETS] = bytemuck::cast_slice(&forward_offsets).to_vec();
    sections[section::FORWARD] = bytemuck::cast_slice(&forward).to_vec();
    sections[section::REVERSE_OFFSETS] = bytemuck::cast_slice(&reverse_offsets).to_vec();
    sections[section::REVERSE] = bytemuck::cast_slice(&reverse).to_vec();
    sections[section::MODULES] = bytemuck::cast_slice(&modules).to_vec();
    sections[section::MODULE_EDGES] = bytemuck::cast_slice(&module_edges).to_vec();
    sections[section::EXTERNALS] = bytemuck::cast_slice(&externals).to_vec();
    sections[section::ENVS] = bytemuck::cast_slice(&envs).to_vec();
    sections[section::FIELDS] = bytemuck::cast_slice(&fields).to_vec();
    sections[section::TOKENS] = bytemuck::cast_slice(&tokens).to_vec();
    sections[section::POSTINGS] = index.postings;
    sections[section::LENGTHS] = bytemuck::cast_slice(&index.lengths).to_vec();
    sections[section::AVERAGES] = bytemuck::cast_slice(&index.averages).to_vec();
    let members: Vec<[u32; 2]> = graph
        .members
        .iter()
        .map(|&(owner, member)| [owner, member])
        .collect();
    sections[section::MEMBERS] = bytemuck::cast_slice(&members).to_vec();
    let stale: Vec<[u32; 3]> = graph
        .stale
        .iter()
        .map(|(node, line, text)| [*node, *line, strings.id(text)])
        .collect();
    sections[section::STALE] = bytemuck::cast_slice(&stale).to_vec();
    let ambiguous: Vec<u32> = graph
        .ambiguous_names
        .iter()
        .map(|name| strings.id(name))
        .collect();
    sections[section::AMBIGUOUS] = bytemuck::cast_slice(&ambiguous).to_vec();
    sections[section::STRINGS] = strings.blob;
    sections[section::STRING_OFFSETS] = bytemuck::cast_slice(&strings.offsets).to_vec();
    let views: Vec<&[u8]> = sections.iter().map(Vec::as_slice).collect();
    container::build(Kind::Derived, &views)
}

/// CSR adjacency by counting sort: `graph.edges` is sorted by source, and a stable placement
/// by target keeps the reverse lists in source order.
fn adjacency(nodes: usize, graph: &Graph, reverse: bool) -> (Vec<u32>, Vec<EdgeRecord>) {
    let mut offsets = vec![0u32; nodes + 1];
    for edge in &graph.edges {
        let from = if reverse { edge.target } else { edge.source };
        offsets[from as usize + 1] += 1;
    }
    for index in 1..offsets.len() {
        offsets[index] += offsets[index - 1];
    }
    let mut cursor: Vec<u32> = offsets[..nodes].to_vec();
    let mut records = vec![EdgeRecord::zeroed(); graph.edges.len()];
    for edge in &graph.edges {
        let (from, to) = if reverse {
            (edge.target, edge.source)
        } else {
            (edge.source, edge.target)
        };
        let slot = &mut cursor[from as usize];
        records[*slot as usize] = EdgeRecord {
            node: to,
            line: edge.line,
            kind: edge.kind as u8,
            confidence: edge.confidence as u8,
            pad: [0; 2],
        };
        *slot += 1;
    }
    (offsets, records)
}

/// Caller and user names kept per symbol for the `used_by` search field.
const USED_BY_NAMES: usize = 16;

struct SearchIndex {
    tokens: Vec<(String, u32, u32)>,
    postings: Vec<u8>,
    lengths: Vec<LengthRecord>,
    averages: [f32; FIELDS],
}

/// Postings of one contiguous node range: token -> postings in node order.
struct Chunk {
    lists: FastMap<String, Vec<PostingRecord>>,
}

fn search_index(graph: &Graph) -> SearchIndex {
    let nodes = graph.node_count();
    let symbol_count = graph.symbols.len();
    // Names of the callers and users of each symbol, distinct, at most USED_BY_NAMES.
    let mut used_by: Vec<String> = vec![String::new(); symbol_count];
    for edge in &graph.edges {
        let (source, target) = (edge.source as usize, edge.target as usize);
        if !matches!(edge.kind, EdgeKind::Calls | EdgeKind::Uses)
            || source >= symbol_count
            || target >= symbol_count
            || source == target
        {
            continue;
        }
        let names = &mut used_by[target];
        let name = &graph.symbols[source].name;
        if names.split(' ').count() < USED_BY_NAMES && !names.split(' ').any(|known| known == name)
        {
            if !names.is_empty() {
                names.push(' ');
            }
            names.push_str(name);
        }
    }
    // Node texts per field; files first so symbols can take their file's path length.
    let texts = |node: usize| -> [&str; FIELDS] {
        if node >= symbol_count {
            let file = &graph.files[node - symbol_count];
            let mut fields = [""; FIELDS];
            fields[search::NAME] = file.path.rsplit('/').next().unwrap_or(&file.path);
            fields[search::PATH] = &file.path;
            fields[search::DOC] = &file.doc;
            // Config key paths and env names weigh like a signature.
            fields[search::SIGNATURE] = &file.keys;
            fields[search::BODY] = &file.terms;
            return fields;
        }
        let symbol = &graph.symbols[node];
        // `mod name;` declarations repeat the module file's own path match.
        if symbol.kind == SymKind::Mod && symbol.line == symbol.end_line {
            return [""; FIELDS];
        }
        let parent = symbol
            .qualified
            .strip_suffix(symbol.name.as_str())
            .unwrap_or_default();
        let mut fields = [""; FIELDS];
        fields[search::NAME] = &symbol.name;
        fields[search::PARENT] = parent;
        fields[search::SIGNATURE] = &symbol.signature;
        fields[search::DOC] = &symbol.doc;
        fields[search::BODY] = &symbol.terms;
        fields[search::USED_BY] = &used_by[node];
        fields
    };
    const CHUNK: usize = 4096;
    let chunk_results: Vec<(Chunk, Vec<LengthRecord>)> = (0..nodes.div_ceil(CHUNK))
        .into_par_iter()
        .map(|chunk| {
            let range = chunk * CHUNK..((chunk + 1) * CHUNK).min(nodes);
            let mut lists: FastMap<String, Vec<PostingRecord>> = FastMap::default();
            let mut lengths = Vec::with_capacity(range.len());
            let mut buffer = String::new();
            for node in range {
                let mut record = LengthRecord {
                    lengths: [0; FIELDS],
                };
                for (field, text) in texts(node).iter().enumerate() {
                    let mut count = 0u8;
                    let mut add = |token: &str| {
                        count = count.saturating_add(1);
                        let list = match lists.get_mut(token) {
                            Some(list) => list,
                            None => lists.entry(token.to_owned()).or_default(),
                        };
                        match list.last_mut() {
                            Some(last) if last.node == node as u32 => last.add(field),
                            _ => {
                                let mut posting = PostingRecord {
                                    node: node as u32,
                                    packed: 0,
                                };
                                posting.add(field);
                                list.push(posting);
                            }
                        }
                    };
                    // Body terms are stored as normalized tokens (src/extract/terms.rs).
                    if field == search::BODY {
                        text.split(' ')
                            .filter(|token| !token.is_empty())
                            .for_each(&mut add);
                    } else {
                        search::for_each_token(text, &mut buffer, add);
                    }
                    record.lengths[field] = count;
                }
                lengths.push(record);
            }
            (Chunk { lists }, lengths)
        })
        .collect();
    let mut lengths: Vec<LengthRecord> = Vec::with_capacity(nodes);
    let mut merged: FastMap<String, Vec<PostingRecord>> = FastMap::default();
    for (chunk, chunk_lengths) in chunk_results {
        lengths.extend(chunk_lengths);
        for (token, list) in chunk.lists {
            // Chunks cover ascending node ranges, so appending keeps lists sorted by node.
            merged.entry(token).or_default().extend(list);
        }
    }
    for symbol in 0..symbol_count {
        let file_node = symbol_count + graph.symbols[symbol].file as usize;
        lengths[symbol].lengths[search::PATH] = lengths[file_node].lengths[search::PATH];
    }
    let mut averages = [0f32; FIELDS];
    for (field, average) in averages.iter_mut().enumerate() {
        let total: u64 = lengths
            .iter()
            .map(|record| u64::from(record.lengths[field]))
            .sum();
        *average = if nodes == 0 {
            1.0
        } else {
            (total as f32 / nodes as f32).max(0.01)
        };
    }
    let mut sorted: Vec<(String, Vec<PostingRecord>)> = merged.into_iter().collect();
    sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut tokens = Vec::with_capacity(sorted.len());
    let mut encoder = Encoder::default();
    for (token, list) in sorted {
        tokens.push((token, encoder.bytes.len() as u32, list.len() as u32));
        let mut previous = 0;
        for posting in list {
            encoder.var_u32(posting.node - previous);
            encoder.var_u32(posting.packed);
            previous = posting.node;
        }
    }
    SearchIndex {
        tokens,
        postings: encoder.bytes,
        lengths,
        averages,
    }
}

/// Read-only view of a mapped `derived` file.
pub struct Derived {
    container: Container,
}

impl Derived {
    pub fn open(container: Container) -> Option<Self> {
        let derived = Self { container };
        // Validate every typed section once; accessors then cast without failure paths.
        let valid = derived.cast::<u32>(section::STRING_OFFSETS).is_some()
            && derived.cast::<FileRecord>(section::FILES).is_some()
            && derived.cast::<SymbolRecord>(section::SYMBOLS).is_some()
            && derived.cast::<u32>(section::FORWARD_OFFSETS).is_some()
            && derived.cast::<EdgeRecord>(section::FORWARD).is_some()
            && derived.cast::<u32>(section::REVERSE_OFFSETS).is_some()
            && derived.cast::<EdgeRecord>(section::REVERSE).is_some()
            && derived.cast::<ModuleRecord>(section::MODULES).is_some()
            && derived
                .cast::<ModuleEdgeRecord>(section::MODULE_EDGES)
                .is_some()
            && derived.cast::<ExternalRecord>(section::EXTERNALS).is_some()
            && derived.cast::<EnvRecord>(section::ENVS).is_some()
            && derived.cast::<FieldsRecord>(section::FIELDS).is_some()
            && derived.cast::<TokenRecord>(section::TOKENS).is_some()
            && derived.cast::<LengthRecord>(section::LENGTHS).is_some()
            && derived
                .cast::<f32>(section::AVERAGES)
                .is_some_and(|averages| averages.len() == FIELDS)
            && derived.cast::<[u32; 2]>(section::MEMBERS).is_some()
            && derived.cast::<[u32; 3]>(section::STALE).is_some()
            && derived.cast::<u32>(section::AMBIGUOUS).is_some()
            && derived.container.section_count() == section::COUNT
            && derived.forward_offsets().len() == derived.node_count() + 1
            && derived.reverse_offsets().len() == derived.node_count() + 1
            && derived.lengths().len() == derived.node_count();
        valid.then_some(derived)
    }

    fn cast<T: Pod>(&self, index: usize) -> Option<&[T]> {
        bytemuck::try_cast_slice(self.container.section(index)).ok()
    }

    fn slice<T: Pod>(&self, index: usize) -> &[T] {
        self.cast(index).unwrap_or_default()
    }

    pub fn string(&self, id: u32) -> &str {
        let offsets = self.slice::<u32>(section::STRING_OFFSETS);
        let (Some(&start), Some(&end)) = (offsets.get(id as usize), offsets.get(id as usize + 1))
        else {
            return "";
        };
        let blob = self.container.section(section::STRINGS);
        blob.get(start as usize..end as usize)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default()
    }

    pub fn files(&self) -> &[FileRecord] {
        self.slice(section::FILES)
    }

    pub fn symbols(&self) -> &[SymbolRecord] {
        self.slice(section::SYMBOLS)
    }

    pub fn node_count(&self) -> usize {
        self.files().len() + self.symbols().len()
    }

    fn forward_offsets(&self) -> &[u32] {
        self.slice(section::FORWARD_OFFSETS)
    }

    fn reverse_offsets(&self) -> &[u32] {
        self.slice(section::REVERSE_OFFSETS)
    }

    /// Outgoing edges of a node (`node` field = target).
    pub fn outgoing(&self, node: u32) -> &[EdgeRecord] {
        self.adjacent(node, section::FORWARD, self.forward_offsets())
    }

    /// Incoming edges of a node (`node` field = source).
    pub fn incoming(&self, node: u32) -> &[EdgeRecord] {
        self.adjacent(node, section::REVERSE, self.reverse_offsets())
    }

    fn adjacent(&self, node: u32, list: usize, offsets: &[u32]) -> &[EdgeRecord] {
        let (Some(&start), Some(&end)) =
            (offsets.get(node as usize), offsets.get(node as usize + 1))
        else {
            return &[];
        };
        self.slice::<EdgeRecord>(list)
            .get(start as usize..end as usize)
            .unwrap_or_default()
    }

    pub fn modules(&self) -> &[ModuleRecord] {
        self.slice(section::MODULES)
    }

    pub fn module_edges(&self) -> &[ModuleEdgeRecord] {
        self.slice(section::MODULE_EDGES)
    }

    pub fn externals(&self) -> &[ExternalRecord] {
        self.slice(section::EXTERNALS)
    }

    pub fn envs(&self) -> &[EnvRecord] {
        self.slice(section::ENVS)
    }

    pub fn fields(&self) -> &[FieldsRecord] {
        self.slice(section::FIELDS)
    }

    pub fn tokens(&self) -> &[TokenRecord] {
        self.slice(section::TOKENS)
    }

    /// Postings of a token, by binary search over the sorted token table.
    pub fn postings(&self, token: &str) -> Vec<PostingRecord> {
        let tokens = self.tokens();
        let Ok(found) = tokens.binary_search_by(|record| self.string(record.token).cmp(token))
        else {
            return Vec::new();
        };
        let record = tokens[found];
        let bytes = self
            .container
            .section(section::POSTINGS)
            .get(record.start as usize..)
            .unwrap_or_default();
        let mut decoder = Decoder::new(bytes);
        let mut list = Vec::with_capacity(record.count as usize);
        let mut node = 0u32;
        for _ in 0..record.count {
            let (Ok(delta), Ok(packed)) = (decoder.var_u32(), decoder.var_u32()) else {
                break;
            };
            node = node.wrapping_add(delta);
            list.push(PostingRecord { node, packed });
        }
        list
    }

    /// Members (methods, associated items) of a type symbol.
    pub fn members(&self, owner: u32) -> impl Iterator<Item = u32> + '_ {
        let pairs = self.slice::<[u32; 2]>(section::MEMBERS);
        let start = pairs.partition_point(|pair| pair[0] < owner);
        pairs[start..]
            .iter()
            .take_while(move |pair| pair[0] == owner)
            .map(|pair| pair[1])
    }

    /// Stale doc references: (referring node, line, text string id).
    pub fn stale(&self) -> &[[u32; 3]] {
        self.slice(section::STALE)
    }

    /// Whether some reference to `name` stayed ambiguous.
    pub fn is_ambiguous_name(&self, name: &str) -> bool {
        self.slice::<u32>(section::AMBIGUOUS)
            .iter()
            .any(|&id| self.string(id) == name)
    }

    pub fn lengths(&self) -> &[LengthRecord] {
        self.slice(section::LENGTHS)
    }

    pub fn averages(&self) -> [f32; FIELDS] {
        let mut averages = [1.0; FIELDS];
        for (slot, value) in averages
            .iter_mut()
            .zip(self.slice::<f32>(section::AVERAGES))
        {
            *slot = *value;
        }
        averages
    }
}

impl SymbolRecord {
    pub fn kind(&self) -> SymKind {
        SymKind::from_u8(self.kind).unwrap_or(SymKind::Fn)
    }
}

impl FileRecord {
    pub fn role(&self) -> Role {
        Role::from_u8(self.role).unwrap_or(Role::Other)
    }

    pub fn file_type(&self) -> Option<FileType> {
        FileType::from_u8(self.file_type)
    }
}

impl EdgeRecord {
    pub fn kind(&self) -> EdgeKind {
        EdgeKind::from_u8(self.kind).unwrap_or(EdgeKind::Uses)
    }

    pub fn is_exact(&self) -> bool {
        self.confidence == Confidence::Exact as u8
    }
}
