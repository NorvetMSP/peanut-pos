output "vpc_id" {
  value       = aws_vpc.this.id
  description = "VPC ID"
}

output "postgres_endpoint" {
  value       = aws_db_instance.postgres.address
  description = "Postgres endpoint DNS"
}

output "redis_endpoint" {
  value       = aws_elasticache_cluster.redis.cache_nodes[0].address
  description = "Redis primary endpoint"
}
