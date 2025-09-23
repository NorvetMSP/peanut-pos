#!/bin/sh
set -eu

vault_fetch() {
  path="$1"
  if [ "${VAULT_ENABLED:-1}" != "1" ]; then
    return 0
  fi
  addr="${VAULT_ADDR:-http://vault:8200}"
  token="${VAULT_TOKEN:-root}"
  retries=${VAULT_RETRIES:-5}
  if ! command -v curl >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
    echo "Vault fetch skipped: curl or jq not available" >&2
    return 0
  fi
  while [ "$retries" -gt 0 ]; do
    response=$(curl -sS --fail -H "X-Vault-Token: $token" "$addr/v1/$path" 2>/dev/null || true)
    if [ -n "$response" ]; then
      exports=$(echo "$response" | jq -r '.data.data | to_entries[] | @sh "export \(.key)=\(.value)"' 2>/dev/null || true)
      if [ -n "$exports" ]; then
        eval "$exports"
        echo "Loaded secrets from $path" >&2
        return 0
      fi
    fi
    retries=$((retries - 1))
    sleep 1
  done
  echo "Vault fetch skipped: unable to load $path" >&2
}

vault_fetch "secret/data/novapos/auth"
vault_fetch "secret/data/novapos/integration-gateway"

exec "$@"
