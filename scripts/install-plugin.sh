#!/usr/bin/env bash

set -e

VERSION="0.1.5"

case "$(uname -s)" in

   Darwin)
     echo '== Installing plugin for Mac OSX =='
     mkdir -p ~/.pact/plugins/protobuf-${VERSION}
     wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-plugin.json -O ~/.pact/plugins/protobuf-${VERSION}/pact-plugin.json
     if [ "$(uname -m)" == "arm64" ]; then
        wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-protobuf-plugin-osx-aarch64.gz -O ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.gz
     else
        wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-protobuf-plugin-osx-x86_64.gz -O ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.gz
     fi
     gunzip -N -f ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.gz
     chmod +x ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin
     ;;

   Linux)
     echo '== Installing plugin for Linux =='
     mkdir -p ~/.pact/plugins/protobuf-${VERSION}
     wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-plugin.json -O ~/.pact/plugins/protobuf-${VERSION}/pact-plugin.json
     wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-protobuf-plugin-linux-x86_64.gz -O ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.gz
     gunzip -N -f ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.gz
     chmod +x ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin
     ;;

   CYGWIN*|MINGW32*|MSYS*|MINGW*)
     echo '== Installing plugin for MS Windows =='
     mkdir -p ~/.pact/plugins/protobuf-${VERSION}
     wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-plugin.json -O ~/.pact/plugins/protobuf-${VERSION}/pact-plugin.json
     wget https://github.com/pactflow/pact-protobuf-plugin/releases/download/v-${VERSION}/pact-protobuf-plugin-windows-x86_64.exe.gz -O ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.exe.gz
     gunzip -N -f ~/.pact/plugins/protobuf-${VERSION}/pact-protobuf-plugin.exe.gz
     ;;

   *)
     echo "ERROR: $(uname -s) is not a supported operating system"
     exit 1
     ;;
esac
