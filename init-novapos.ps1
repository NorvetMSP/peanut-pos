
<#
 NovaPOS Repo Bootstrap/Repair Script
 Usage: ./init-novapos.ps1

 Idempotent: only creates missing files/folders and appends safe defaults.
 It detects existing workspace/frontends and avoids overwriting user content.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Ensure-Directory {
  param([string]$Path)
  if (-not (Test-Path -LiteralPath $Path)) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
  }
}

function Ensure-File {
  param(
    [string]$Path,
    [string]$Content
  )
  if (-not (Test-Path -LiteralPath $Path)) {
    $Content | Out-File -LiteralPath $Path -Encoding utf8
    return $true
  }
  return $false
}

function AddLinesIfMissing {
  param(
    [string]$Path,
    [string[]]$Lines
  )
  if (-not (Test-Path -LiteralPath $Path)) {
    $Lines -join "`n" | Out-File -LiteralPath $Path -Encoding utf8
    return
  }
  $existing = Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue
  $toAdd = @()
  foreach ($line in $Lines) {
    if ($existing -notcontains $line) { $toAdd += $line }
  }
  if ($toAdd.Count -gt 0) {
    Add-Content -LiteralPath $Path -Value ("`n" + ($toAdd -join "`n"))
  }
}

Write-Host "Initializing/repairing NovaPOS repository (non-destructive)"

# Git init (if needed)
if (-not (Test-Path -LiteralPath '.git')) {
  git init | Out-Null
  Write-Host "Initialized git repo"
}

# Top-level files
AddLinesIfMissing -Path '.gitignore' -Lines @(
  '# Ignore files',
  'node_modules/',
  'dist/',
  'target/',
  '*.log',
  '.env',
  '.terraform/',
  '*.tfstate',
  '.vscode/',
  '.idea/',
  '.DS_Store'
)

Ensure-File -Path 'LICENSE' -Content @'
MIT License

Copyright (c) 2025

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:
... 
'@

Ensure-File -Path 'README.md' -Content @'
# NovaPOS

Cloud-native, multi-tenant, offline-capable Point of Sale platform.

## Layout
- services/ (Rust microservices)
- frontends/ (React apps)
- infra/ (Terraform, K8s manifests)
- local/ (docker-compose for Postgres, Redis, Kafka)
'@ | Out-Null

# Rust workspace
Ensure-Directory services

$workspaceToml = Join-Path services 'Cargo.toml'
$defaultMembers = @(
  'auth-service',
  'order-service',
  'product-service',
  'payment-service',
  'integration-gateway',
  'analytics-service'
)

if (-not (Test-Path -LiteralPath $workspaceToml)) {
  @"
[workspace]
members = [
  "auth-service",
  "order-service",
  "product-service",
  "payment-service",
  "integration-gateway",
  "analytics-service"
]
resolver = "2"
"@ | Out-File -LiteralPath $workspaceToml -Encoding utf8
  $members = $defaultMembers
} else {
  # Parse members from existing services/Cargo.toml (simple heuristic)
  $content = Get-Content -LiteralPath $workspaceToml -Raw
  $members = @()
  if ($content -match '(?s)members\s*=\s*\[(.*?)\]') {
    $inner = $Matches[1]
    $svcMatches = [regex]::Matches($inner, '"([^"]+)"')
    foreach ($m in $svcMatches) { $members += $m.Groups[1].Value }
  }
  if (-not $members -or $members.Count -eq 0) { $members = $defaultMembers }
}

foreach ($svc in $members) {
  $svcPath = Join-Path services $svc
  Ensure-Directory $svcPath
  Ensure-Directory (Join-Path $svcPath 'src')

  $svcCargo = Join-Path $svcPath 'Cargo.toml'
  Ensure-File -Path $svcCargo -Content @"
[package]
name = "${svc}"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt","env-filter"] }
anyhow = "1"
thiserror = "1"
"@ | Out-Null

  $mainRs = Join-Path $svcPath 'src/main.rs'
  if (-not (Test-Path -LiteralPath $mainRs)) {
@"
use axum::{routing::get, Router};
use std::net::SocketAddr;

async fn health() -> &'static str { "ok" }

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let app = Router::new().route("/healthz", get(health));
    let addr = SocketAddr::from(([0,0,0,0], 8080));
  println!("starting ${svc} on {addr}");
    axum::Server::bind(&addr).serve(app.into_make_service()).await?;
    Ok(())
}
"@ | Out-File -LiteralPath $mainRs -Encoding utf8
  }
}

# Frontends (create only if missing)
Ensure-Directory 'frontends'

function Ensure-Frontend {
  param([string]$Name)
  $appPath = Join-Path 'frontends' $Name
  if (Test-Path -LiteralPath (Join-Path $appPath 'package.json')) { return }
  Ensure-Directory (Join-Path $appPath 'src')

  @"
{
  "name": "$Name",
  "version": "0.0.1",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^18.2.0",
    "react-dom": "^18.2.0"
  },
  "devDependencies": {
    "vite": "^5.2.0",
    "typescript": "^5.4.0",
    "@types/react": "^18.2.0",
    "@types/react-dom": "^18.2.0"
  }
}
"@ | Out-File -LiteralPath (Join-Path $appPath 'package.json') -Encoding utf8

  @"
import React from "react";
import { createRoot } from "react-dom/client";

function App() {
  return <h1>NovaPOS - ${Name}</h1>;
}

const root = document.getElementById("root");
createRoot(root).render(<App />);
"@ | Out-File -LiteralPath (Join-Path $appPath 'src/main.tsx') -Encoding utf8

  @'
<!doctype html>
<html>
  <head><meta charset="utf-8"/><title>NovaPOS</title></head>
  <body><div id="root"></div><script type="module" src="/src/main.tsx"></script></body>
</html>
'@ | Out-File -LiteralPath (Join-Path $appPath 'index.html') -Encoding utf8
}

# Only scaffold missing apps; do not overwrite existing pos-app
if (-not (Test-Path -LiteralPath 'frontends/pos-app/package.json')) {
  Ensure-Frontend -Name 'pos-app'
}

# Optional: create admin-portal only if requested via env var
if ($env:ADD_ADMIN_PORTAL -eq '1' -and -not (Test-Path -LiteralPath 'frontends/admin-portal/package.json')) {
  Ensure-Frontend -Name 'admin-portal'
}

# Local docker-compose
Ensure-Directory 'local/docker-compose'
Ensure-File -Path 'local/docker-compose/docker-compose.yml' -Content @'
version: "3.9"
services:
  postgres:
    image: postgres:16
    environment:
      POSTGRES_USER: novapos
      POSTGRES_PASSWORD: novapos
      POSTGRES_DB: novapos
    ports: [ "5432:5432" ]
  redis:
    image: redis:7
    ports: [ "6379:6379" ]
  zookeeper:
    image: bitnami/zookeeper:3.8
    environment:
      - ALLOW_ANONYMOUS_LOGIN=yes
    ports: [ "2181:2181" ]
  kafka:
    image: bitnami/kafka:3.7
    environment:
      - KAFKA_BROKER_ID=1
      - KAFKA_CFG_ZOOKEEPER_CONNECT=zookeeper:2181
      - ALLOW_PLAINTEXT_LISTENER=yes
      - KAFKA_CFG_LISTENERS=PLAINTEXT://:9092
      - KAFKA_CFG_ADVERTISED_LISTENERS=PLAINTEXT://kafka:9092
    ports: [ "9092:9092" ]
'@ | Out-Null

# GitHub Actions CI (create only if missing)
Ensure-Directory '.github/workflows'
Ensure-File -Path '.github/workflows/ci.yml' -Content @'
name: ci
on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace
      - uses: actions/setup-node@v4
        with:
          node-version: 20
      - name: Build pos-app
        if: ${{ hashFiles('frontends/pos-app/package.json') != '' }}
        run: cd frontends/pos-app && npm ci && npm run build
      - name: Build admin-portal
        if: ${{ hashFiles('frontends/admin-portal/package.json') != '' }}
        run: cd frontends/admin-portal && npm ci && npm run build
'@ | Out-Null

Write-Host "✅ NovaPOS repo initialized/updated. Next: run cargo build/test and npm builds as needed."
