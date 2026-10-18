# Run from source (scans ~, or pass a folder: just run ~/repos).
run *path:
    cargo run --release -p rsdisk -- {{path}}

# Build target/RSdisk.app.
bundle:
    tools/bundle.sh

# Build the app and install it in ~/Applications.
install: bundle
    rm -rf ~/Applications/RSdisk.app
    mkdir -p ~/Applications
    cp -R target/RSdisk.app ~/Applications/
