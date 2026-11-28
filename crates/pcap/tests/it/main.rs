//! Integration tests for craft-pcap.

mod dns_discovery_answers;
mod dns_question_class;
mod dns_rdata_names;
mod pcap_dns_name_limit;
mod pcap_dns_txt_lengths;
mod pcap_flow_time_range;
mod pcap_http_host_boundary;
mod pcap_http_methods;
mod pcap_ipv4_lengths;
mod pcap_ipv6_fragments;
mod pcap_ipv6_no_next_header;
mod pcap_ipv6_version;
mod pcap_tcp_ecn;
mod pcapng_simple_payload;
mod regression_pcapng_framing;
mod regression_tcp_dns_framing;
mod regression_tcp_header_length;
mod regression_tls_bounds;
#[path = "../support/mod.rs"]
mod support;
