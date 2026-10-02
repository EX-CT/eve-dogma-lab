#!/usr/bin/env bash
# Build variant K (C# / .NET 8, Native AOT) into ./bin/eve-dogma-k. Installs the .NET 8 SDK into ~/.dotnet if missing.
set -euo pipefail
cd "$(dirname "$0")"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_SKIP_FIRST_TIME_EXPERIENCE=1
if ! command -v dotnet >/dev/null 2>&1; then
  if [ ! -x "$HOME/.dotnet/dotnet" ]; then
    curl -sSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh
    bash /tmp/dotnet-install.sh --channel 8.0 --install-dir "$HOME/.dotnet" >/dev/null
  fi
  export PATH="$HOME/.dotnet:$PATH" DOTNET_ROOT="$HOME/.dotnet"
fi
dotnet publish src/EveDogmaK/EveDogmaK.csproj -c Release -r linux-x64 -o bin --nologo -v quiet
rm -f bin/*.dbg
echo "built $(pwd)/bin/eve-dogma-k"
