pub mod model;
pub mod producer;

pub use model::{AuditEvent, AuditActor, AuditError, AuditResult, AUDIT_EVENT_VERSION, AuditSeverity};
pub use producer::{AuditProducer, AuditProducerConfig, BufferedAuditProducer, extract_actor_from_headers, NoopAuditSink};
// Export real KafkaAuditSink when kafka-producer (or legacy kafka umbrella) enabled; otherwise export stub if kafka-core enabled.
#[cfg(any(feature = "kafka", feature = "kafka-producer"))]
pub use producer::KafkaAuditSink;
#[cfg(all(feature = "kafka-core", not(any(feature = "kafka", feature = "kafka-producer"))))]
pub use producer::KafkaAuditSink; // stub variant
