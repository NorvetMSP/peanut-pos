# Wave 5 Secret Promotion Guide

## Purpose
Staging and production must mirror the credentials now stored in local Vault for auth-service and integration-gateway. This guide documents how to export the vetted key-value pairs and publish them into the secret manager that backs each runtime (Vault, AWS Secrets Manager, Azure Key Vault, Kubernetes, etc.).

## Source of Truth
- Local path: `secret/novapos/auth`
- Local path: `secret/novapos/integration-gateway`
- Expected keys (baseline):
  - `KAFKA_BOOTSTRAP`
  - `REDIS_URL`
  - `SECURITY_MFA_ACTIVITY_TOPIC`
  - `SECURITY_ALERT_TOPIC`
  - Optional webhook bearer/URL values when available

> Update this list if additional keys are introduced in Vault. Treat the local store as the canonical set until the production manager is in sync.

## Step 1: Export the Local Secrets
1. Start the local Vault container (`docker compose up -d vault`) if it is not already running.
2. Export the secret payloads:
   ```powershell
   $env:VAULT_ADDR = 'http://127.0.0.1:8200'
   $env:VAULT_TOKEN = 'root'
   vault kv get -format=json secret/novapos/auth | Out-File tmp-auth.json
   vault kv get -format=json secret/novapos/integration-gateway | Out-File tmp-gateway.json
   ```
3. Review the JSON files to confirm the current key set and values.

## Step 2: Publish to the Production Secret Manager
Choose the method that matches your environment. Below are examples for common platforms.

### HashiCorp Vault (shared cluster)
```bash
VAULT_ADDR=https://vault.prod.example
VAULT_TOKEN=<deployment-token>
vault kv put secret/novapos/auth @tmp-auth.json
vault kv put secret/novapos/integration-gateway @tmp-gateway.json
```

### AWS Secrets Manager
```bash
aws secretsmanager put-secret-value \
  --secret-id prod/novapos/auth \
  --secret-string file://tmp-auth.json
aws secretsmanager put-secret-value \
  --secret-id prod/novapos/integration-gateway \
  --secret-string file://tmp-gateway.json
```

### Kubernetes Secrets (Helm/Kustomize)
```bash
kubectl create secret generic auth-secrets \
  --namespace novapos-prod \
  --from-literal=KAFKA_BOOTSTRAP=$(jq -r '.data.KAFKA_BOOTSTRAP' tmp-auth.json) \
  --from-literal=SECURITY_MFA_ACTIVITY_TOPIC=$(jq -r '.data.SECURITY_MFA_ACTIVITY_TOPIC' tmp-auth.json) \
  --dry-run=client -o yaml > overlay/auth-secrets.yaml
```
Repeat for the gateway secret, then commit the rendered manifests.

## Step 3: Update Application Manifests
- Helm: set `authService.secretRef` and `integrationGateway.secretRef` to the production secret names.
- ECS/Fargate: update task definition environment overrides to point at the new secrets ARNs.
- Docker Swarm or bare metal: update compose files or systemd unit drop-ins with the resolved secret file paths.

## Step 4: Validate in Staging and Production
1. Redeploy staging with the new secret references.
2. Confirm the services boot without missing environment errors.
3. Hit `/metrics` on each service to ensure the secret-dependent components (Kafka, Redis, Webhooks) initialize successfully.
4. Repeat the deployment in production after staging validation passes.

## Cleanup and Tracking
- Remove the temporary `tmp-auth.json` and `tmp-gateway.json` files.
- Record the promotion in the change log or ticket, linking to the deployment run.
- Update the Jira Wave 5 epic checklist to mark "Secrets mirrored to prod" complete.
