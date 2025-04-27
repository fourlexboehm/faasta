# Faasta CLI

Command-line interface for the Faasta (Functions as a Service) platform.

## Features

- Initialize new Faasta functions
- Deploy functions to Faasta servers
- Run functions locally for testing
- Authentication with GitHub OAuth

## Installation

```
cargo install faasta-cli
```

## Usage

```
cargo faasta init       # Initialize a new Faasta function in current directory
cargo faasta new NAME   # Create a new Faasta function in a new directory
cargo faasta build      # Build the function for deployment
cargo faasta deploy     # Deploy the function to a Faasta server
cargo faasta run        # Run the function locally for testing
cargo faasta login      # Authenticate with GitHub
cargo faasta list       # List all deployed functions
cargo faasta metrics    # View metrics for your deployed functions
cargo faasta invoke     # Invoke a deployed function
cargo faasta unpublish  # Unpublish a function from the server
```

## Configuration

The CLI uses a configuration file located at `~/.faasta/config.json`.

## License

See the main project repository for license information.