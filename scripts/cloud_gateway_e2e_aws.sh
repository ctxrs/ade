#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
core_dir="$root_dir/core"

region="${AWS_REGION:-${AWS_DEFAULT_REGION:-us-east-1}}"
export AWS_REGION="$region"
export AWS_DEFAULT_REGION="$region"

if ! command -v aws >/dev/null 2>&1; then
  echo "[e2e] aws cli not found" >&2
  exit 1
fi

account_id="$(aws sts get-caller-identity --query Account --output text --region "$region")"
bucket_name="${AWS_E2E_ARTIFACT_BUCKET:-ctx-cloud-gateway-e2e-${account_id}-${region}}"

vpc_id="${AWS_E2E_VPC_ID:-}"
if [ -z "$vpc_id" ]; then
  vpc_id="$(aws ec2 describe-vpcs --filters Name=isDefault,Values=true --query 'Vpcs[0].VpcId' --output text --region "$region")"
fi
if [ -z "$vpc_id" ] || [ "$vpc_id" = "None" ]; then
  vpc_id="$(aws ec2 describe-vpcs --query 'Vpcs[0].VpcId' --output text --region "$region")"
fi
if [ -z "$vpc_id" ] || [ "$vpc_id" = "None" ]; then
  echo "[e2e] failed to resolve VPC" >&2
  exit 1
fi

subnet_id="${AWS_E2E_SUBNET_ID:-}"
if [ -z "$subnet_id" ]; then
  subnet_id="$(aws ec2 describe-subnets --filters Name=vpc-id,Values="$vpc_id" Name=map-public-ip-on-launch,Values=true --query 'Subnets[0].SubnetId' --output text --region "$region")"
fi
if [ -z "$subnet_id" ] || [ "$subnet_id" = "None" ]; then
  subnet_id="$(aws ec2 describe-subnets --filters Name=vpc-id,Values="$vpc_id" --query 'Subnets[0].SubnetId' --output text --region "$region")"
fi
if [ -z "$subnet_id" ] || [ "$subnet_id" = "None" ]; then
  echo "[e2e] failed to resolve subnet" >&2
  exit 1
fi

sg_id="${AWS_E2E_SECURITY_GROUP_ID:-}"
if [ -z "$sg_id" ]; then
  sg_id="$(aws ec2 describe-security-groups --filters Name=vpc-id,Values="$vpc_id" Name=group-name,Values=ctx-worker-gateway-e2e --query 'SecurityGroups[0].GroupId' --output text --region "$region")"
fi
if [ -z "$sg_id" ] || [ "$sg_id" = "None" ]; then
  sg_id="$(aws ec2 create-security-group --group-name ctx-worker-gateway-e2e --description "ctx worker gateway e2e" --vpc-id "$vpc_id" --query 'GroupId' --output text --region "$region")"
  aws ec2 authorize-security-group-ingress --group-id "$sg_id" --protocol tcp --port 8787 --cidr 0.0.0.0/0 --region "$region" >/dev/null 2>&1 || true
fi

if [ -n "${AWS_E2E_SSH_PRIVATE_KEY:-}" ]; then
  key_name="${AWS_E2E_SSH_KEY_NAME:-ctx-gateway-e2e}"
  key_file="$(mktemp)"
  key_pub="${key_file}.pub"
  printf '%s' "$AWS_E2E_SSH_PRIVATE_KEY" > "$key_file"
  chmod 600 "$key_file"
  trap 'rm -f "$key_file" "$key_pub"' EXIT
  if ! aws ec2 describe-key-pairs --key-names "$key_name" --region "$region" >/dev/null 2>&1; then
    if command -v ssh-keygen >/dev/null 2>&1; then
      ssh-keygen -y -f "$key_file" > "$key_pub"
      aws ec2 import-key-pair --key-name "$key_name" --public-key-material "fileb://$key_pub" --region "$region" >/dev/null
    else
      echo "[e2e] ssh-keygen not found; cannot import SSH key" >&2
      exit 1
    fi
  fi
  export AWS_SSH_KEY_NAME="$key_name"
  export AWS_SSH_USER="${AWS_E2E_SSH_USER:-ubuntu}"
  aws ec2 authorize-security-group-ingress --group-id "$sg_id" --protocol tcp --port 22 --cidr 0.0.0.0/0 --region "$region" >/dev/null 2>&1 || true
fi

if ! aws s3api head-bucket --bucket "$bucket_name" --region "$region" >/dev/null 2>&1; then
  if [ "$region" = "us-east-1" ]; then
    aws s3api create-bucket --bucket "$bucket_name" --region "$region" >/dev/null
  else
    aws s3api create-bucket --bucket "$bucket_name" --region "$region" --create-bucket-configuration LocationConstraint="$region" >/dev/null
  fi
fi

ami_id="${AWS_E2E_AMI_ID:-}"
if [ -z "$ami_id" ]; then
  ami_id="$(aws ssm get-parameter --name /aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id --query 'Parameter.Value' --output text --region "$region")"
fi

export AWS_SUBNET_ID="$subnet_id"
export AWS_SECURITY_GROUP_ID="$sg_id"
export AWS_ARTIFACT_BUCKET="$bucket_name"
export AWS_AMI_ID="$ami_id"
export AWS_GATEWAY_INSTANCE_TYPE="${AWS_E2E_GATEWAY_INSTANCE_TYPE:-t3.small}"
export AWS_WORKER_INSTANCE_TYPE="${AWS_E2E_WORKER_INSTANCE_TYPE:-t3.small}"

export CTX_E2E_TIER=3
export CTX_E2E_PROVIDER_ID="${CTX_E2E_PROVIDER_ID:-codex}"
export CTX_E2E_MODEL_ID="${CTX_E2E_MODEL_ID:-gpt-5.2-codex}"
export CTX_E2E_KEEP_RESOURCES="${CTX_E2E_KEEP_RESOURCES:-0}"

repo_dir="${CTX_E2E_WORKSPACE_ROOT:-/tmp/ctx-e2e-public-repo}"
if [ ! -d "$repo_dir/.git" ]; then
  rm -rf "$repo_dir"
  git clone https://github.com/octocat/Hello-World.git "$repo_dir"
fi
export CTX_E2E_WORKSPACE_ROOT="$repo_dir"

cd "$core_dir"

CARGO_TARGET_DIR=target cargo build -p ctx-worker-gateway -p ctx-worker-shim

export CTX_WORKER_GATEWAY_BIN="$core_dir/target/debug/ctx-worker-gateway"
export CTX_WORKER_SHIM_BIN="$core_dir/target/debug/ctx-worker-shim"

CARGO_TARGET_DIR=target cargo test -p ctx-http --test cloud_gateway_aws_e2e -- --ignored --nocapture --test-threads=1
