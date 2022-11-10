pub struct GGCATConfig {
    /// Directory for temporary files
    temp_dir: String,
    /// Maximum suggested memory usage (GB)
    /// The tool will try use only up to this GB of memory to store temporary files
    /// without writing to disk. This usage does not include the needed memory for the processing steps.
    /// GGCAT can allocate extra memory for files if the current memory is not enough to complete the current operation
    memory: f64,
    /// Use all the given memory before writing to disk
    prefer_memory: bool,
}

struct GGCATInstance {}

impl GGCATInstance {
    pub fn initialize(config: GGCATConfig) {}
}

pub fn build_graph(
    // The input files
    input_files: Vec<String>,

    // The output file
    output_file: String,

    // Specifies the k-mers length
    kmer_length: usize,
    // The threads to be used
    threads_count: usize,
    // Treats reverse complementary kmers as different
    forward_only: bool,
    // Overrides the default m-mers (minimizers) length
    minimizer_length: usize,

    // Enable colors
    colors: bool,

    // Minimum multiplicity required to keep a kmer
    min_multiplicity: usize,

    // Generate maximal unitigs connections references, in BCALM2 format L:<+/->:<other id>:<+/->
    generate_maximal_unitigs_links: bool,
    // // Generate greedy matchtigs instead of maximal unitigs
    // greedy_matchtigs: bool,
    //
    // // Generate eulertigs instead of maximal unitigs
    // eulertigs: bool,
    //
    // // Generate pathtigs instead of maximal unitigs
    // pathtigs: bool,
) {
}

pub fn query_graph(
    // The input graph
    input_graph: String,
    // The input query as a .fasta file
    input_query: String,

    // The output file
    output_file_prefix: String,

    // Specifies the k-mers length
    kmer_length: usize,
    // The threads to be used
    threads_count: usize,
    // Treats reverse complementary kmers as different
    forward_only: bool,
    // Overrides the default m-mers (minimizers) length
    minimizer_length: usize,

    // Enable colors
    colors: bool,
) {
}

fn read_graph(
    // The input graph
    input_graph: String,
    // Enable colors
    colors: bool,
    output_file_prefix: String,
) {
}