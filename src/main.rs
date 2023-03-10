#[macro_use]
extern crate prettytable;
use std::error::Error;
use std::fmt::Display;
use std::path::PathBuf;

use depchk::npm::PackageJson;
use depchk::*;

use reqwest::Client;

use clap::{Parser, ValueEnum};

use prettytable::Table;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum OutputTypes {
    Table,
    Json,
    Yaml,
    Csv,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum DependencyType {
    Npm,
}

#[derive(Debug, Default)]
struct DependencyCheckErrors {
    errors: Vec<Box<dyn Error>>,
    msg: String,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// If true, also checks the dev dependencies for updates
    #[arg(short, long)]
    dev: bool,

    /// What type of dependency is being parsed
    dependency: DependencyType,

    /// Path to the dependency file. If not given, assumes that it is in the current directory.
    /// The actual default is determined by the dependency type. For example, for the npm dependency type,
    /// the default is "package.json"
    file: Option<PathBuf>,

    /// The display type of the output
    #[arg(value_enum, short, long)]
    output: Option<OutputTypes>,
}

impl Default for OutputTypes {
    fn default() -> Self {
        OutputTypes::Table
    }
}

impl DependencyCheckErrors {
    fn new(err: Vec<Box<dyn Error>>) -> Self {
        let msg = err
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<String>>()
            .join("\n");
        DependencyCheckErrors { errors: err, msg }
    }

    fn join(&mut self, mut err: DependencyCheckErrors) {
        self.errors.append(&mut err.errors);
    }
}

impl Display for DependencyCheckErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.msg)
    }
}

impl Error for DependencyCheckErrors {
    fn description(&self) -> &str {
        &self.msg
    }
}

fn handle_dependency_result(
    results: Vec<DependencyMismatchResult>,
) -> (Vec<VersionMismatch>, DependencyCheckErrors) {
    let (mismatches, errs): (Vec<_>, Vec<_>) = results.into_iter().partition(|res| res.is_ok());
    let mismatches = mismatches.into_iter().map(|res| res.unwrap()).collect();

    let errs = errs.into_iter().map(|res| res.unwrap_err()).collect();

    (mismatches, DependencyCheckErrors::new(errs))
}

fn print_table_mismatches(mismatches: &Mismatches) {
    let mut table = Table::new();

    table.set_titles(row![b->"Package Name", b->"Version Constraint", b->"Latest Version"]);

    for mismatch in &mismatches.dependencies {
        let (name, constraint, version) = mismatch.destruct();

        table.add_row(row![FG->name, FB->constraint, FR->version]);
    }

    if mismatches.dev_dependencies.is_some() {
        table.add_row(row![bH3->"Dev Dependencies"]);

        for mismatch in mismatches.dev_dependencies.as_ref().unwrap() {
            let (name, constraint, version) = mismatch.destruct();

            table.add_row(row![FG->name, FB->constraint, FR->version]);
        }
    }

    table.printstd();
}

fn print_csv_mismatches(mismatches: &Mismatches) {
    for mismatch in &mismatches.dependencies {
        let (name, constraint, version) = mismatch.destruct();

        println!("{},{},{}", name, constraint, version);
    }

    if mismatches.dev_dependencies.is_none() {
        return;
    }
    for mismatch in mismatches.dev_dependencies.as_ref().unwrap() {
        let (name, constraint, version) = mismatch.destruct();

        println!("{},{},{}", name, constraint, version);
    }
}

async fn to_mismatches<T: Dependency>(
    dependencies: ProjectDependencies<T>,
    include_dev_dependencies: bool,
) -> Result<(Mismatches, DependencyCheckErrors), Box<dyn Error>> {
    let client = Client::builder().build()?;
    let (mismatches, mut err) =
        handle_dependency_result(dependencies.check_dependencies(&client).await);

    let (dev_mismatches, dev_err) = {
        if include_dev_dependencies {
            let (mismatch, err) =
                handle_dependency_result(dependencies.check_dev_dependencies(&client).await);
            (Some(mismatch), err)
        } else {
            (None, DependencyCheckErrors::default())
        }
    };

    let all_mismatches = Mismatches {
        dependencies: mismatches,
        dev_dependencies: dev_mismatches,
    };

    err.join(dev_err);

    Ok((all_mismatches, err))
}

async fn depchk(
    path: PathBuf,
    include_dev_dependencies: bool,
    dependency_type: DependencyType,
    output_type: OutputTypes,
) -> Result<(), Box<dyn Error>> {
    let package_json = path.to_str().unwrap();

    let dependencies = match dependency_type {
        DependencyType::Npm => PackageJson::parse_file(package_json)?,
    };

    let (mismatches, err) = to_mismatches(dependencies, include_dev_dependencies).await?;

    match output_type {
        OutputTypes::Table => print_table_mismatches(&mismatches),
        OutputTypes::Json => println!("{}", serde_json::to_string(&mismatches)?),
        OutputTypes::Yaml => println!("{}", serde_yaml::to_string(&mismatches)?),
        OutputTypes::Csv => print_csv_mismatches(&mismatches),
    }

    if !err.errors.is_empty() {
        return Err(Box::new(err));
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    let file = match cli.dependency {
        DependencyType::Npm => cli.file.unwrap_or_else(|| PathBuf::from("package.json")),
    };

    let result = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()
        .expect("Could not build async runtime")
        .block_on(depchk(
            file,
            cli.dev,
            cli.dependency,
            cli.output.unwrap_or_default(),
        ));

    result
}
