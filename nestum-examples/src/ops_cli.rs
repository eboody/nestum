use clap::{Args, Parser, Subcommand};
use nestum::{nested, nestum};

#[derive(Debug, Clone, Args)]
pub struct CreateUserArgs {
    pub email: String,
}

#[derive(Debug, Clone, Args)]
pub struct ResetPasswordArgs {
    pub user_id: u64,
}

#[derive(Debug, Clone, Args)]
pub struct ChargeInvoiceArgs {
    pub invoice_id: u64,
    #[arg(long)]
    pub cents: u64,
}

pub mod command {
    use super::*;

    #[nestum]
    #[derive(Debug, Clone, Subcommand)]
    pub enum User {
        Create(CreateUserArgs),
        Suspend { user_id: u64 },
        ResetPassword(ResetPasswordArgs),
    }

    #[nestum]
    #[derive(Debug, Clone, Subcommand)]
    pub enum Billing {
        Charge(ChargeInvoiceArgs),
        Refund { invoice_id: u64 },
    }
}

#[nestum]
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(subcommand)]
    Users(command::User),
    #[command(subcommand)]
    Billing(command::Billing),
}

#[derive(Debug, Parser)]
#[command(
    name = "ops-console",
    about = "Nested command dispatch demo built with nestum"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command::Enum,
}

impl Cli {
    pub fn run(self) -> String {
        self.command.dispatch()
    }
}

impl Command::Enum {
    pub fn dispatch(self) -> String {
        nested! {
            match self {
                Command::Users::Create(args) => format!("create-user:{}", args.email),
                Command::Users::Suspend { user_id } => format!("suspend-user:{user_id}"),
                Command::Users::ResetPassword(args) => format!("reset-password:{}", args.user_id),
                Command::Billing::Charge(args) => {
                    format!("charge-invoice:{}:{}c", args.invoice_id, args.cents)
                }
                Command::Billing::Refund { invoice_id } => format!("refund-invoice:{invoice_id}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_parses_nested_subcommands() {
        let cli =
            Cli::try_parse_from(["ops-console", "users", "create", "dev@example.com"]).unwrap();

        assert_eq!(cli.run(), "create-user:dev@example.com");
    }

    #[test]
    fn clap_handles_struct_variants_and_dispatch() {
        let cli = Cli::try_parse_from(["ops-console", "billing", "refund", "42"]).unwrap();

        assert_eq!(cli.run(), "refund-invoice:42");
    }

    #[test]
    fn clap_handles_deeper_nested_dispatch() {
        let cli = Cli::try_parse_from(["ops-console", "billing", "charge", "7", "--cents", "1200"])
            .unwrap();

        assert_eq!(cli.run(), "charge-invoice:7:1200c");
    }

    #[test]
    fn dispatch_accepts_nested_constructor_paths_directly() {
        let result = nested! {
            Command::Users::Suspend {
                user_id: 12,
            }
        }
        .dispatch();

        assert_eq!(result, "suspend-user:12");
    }
}
