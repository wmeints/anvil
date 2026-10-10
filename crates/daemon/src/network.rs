//! Network settings of sandboxes: removes the network device of a sandbox whose network is
//! disabled, and otherwise turns the egress rules of a spec into microsandbox's network policy,
//! with TLS interception and an HTTP deny response that tells the user how to allow a blocked
//! host.

use firebrick_spec::{NetworkRule, NetworkSpec};
use microsandbox::MicrosandboxError;
use microsandbox::sandbox::SandboxBuilder;
use microsandbox_network::policy::{
    Action, NetworkPolicy, Rule, RuleBuilder, RuleDestinationBuilder,
};

/// Body of the `403 Forbidden` response to a denied HTTP or HTTPS request. microsandbox
/// replaces `{host}` with the denied host.
pub const DENY_MESSAGE: &str = "firebrick blocked the connection to {host}: the network policy \
     of this sandbox doesn't allow it. To allow it, run `fbk network allow {host}` on the host, \
     outside the sandbox.";

/// Label that records the network settings a sandbox was created with, so an update that
/// doesn't change them can leave the sandbox alone.
pub const RULES_LABEL: &str = "firebrick.network";

/// Returns the value of [`RULES_LABEL`] for the network settings. A disabled network ignores
/// its rules, so it is recorded as `disabled`. Rules that aren't enforced don't change the
/// sandbox, so they are all recorded as `off`.
pub fn rules_label(network: &NetworkSpec) -> String {
    if !network.is_enabled() {
        return "disabled".to_string();
    }

    if !network.enforce {
        return "off".to_string();
    }

    let join = |rules: &[NetworkRule]| {
        rules
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };

    format!(
        "allow={};deny={}",
        join(&network.allow),
        join(&network.deny)
    )
}

/// Adds the network settings to a sandbox that is being created. A disabled network removes
/// the sandbox's network device and ignores the rules. Without `enforce`, the sandbox keeps
/// microsandbox's default policy and no TLS interception.
pub fn add_to_builder(
    builder: SandboxBuilder,
    network: &NetworkSpec,
) -> Result<SandboxBuilder, MicrosandboxError> {
    if !network.is_enabled() {
        return Ok(builder.disable_network());
    }

    if !network.enforce {
        return Ok(builder);
    }

    let policy = policy(network)?;

    // `tls(|t| t)` intercepts TLS on port 443, so HTTPS requests can get the deny response too.
    Ok(builder.network(|n| {
        n.policy(policy)
            .tls(|t| t)
            .http(|h| h.deny_response(true).deny_message(DENY_MESSAGE))
    }))
}

/// Returns the policy that enforces the rules: all deny rules, then all allow rules, then DNS
/// to the sandbox's resolver, and deny for any other egress. Rules match first-come, so a deny
/// rule wins over an allow rule regardless of where either is written. microsandbox also
/// matches domain rules against DNS queries, so a host denied by a domain rule doesn't
/// resolve, while every other name does.
pub fn policy(network: &NetworkSpec) -> Result<NetworkPolicy, MicrosandboxError> {
    let mut policy = NetworkPolicy::builder()
        .default_egress(Action::Deny)
        .egress(|r| {
            for rule in &network.deny {
                add_rule(r.deny(), rule);
            }
            for rule in &network.allow {
                add_rule(r.allow(), rule);
            }
            r
        })
        .build()?;

    policy.rules.push(Rule::allow_dns());

    Ok(policy)
}

/// Commits a rule with the destination of the spec's rule.
fn add_rule<'a>(
    destination: RuleDestinationBuilder<'a>,
    rule: &NetworkRule,
) -> &'a mut RuleBuilder {
    match rule {
        NetworkRule::Domain(domain) => destination.domain(domain),
        NetworkRule::DomainSuffix(domain) => destination.domain_suffix(domain),
        NetworkRule::Ip(address) => destination.ip(address.to_string()),
        NetworkRule::Cidr { .. } => destination.cidr(rule.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Secret;
    use microsandbox::sandbox::{Sandbox, SandboxConfig};
    use microsandbox_network::policy::{Destination, Direction};

    fn rules(values: &[&str]) -> Vec<NetworkRule> {
        values.iter().map(|value| value.parse().unwrap()).collect()
    }

    fn enforced(allow: &[&str], deny: &[&str]) -> NetworkSpec {
        NetworkSpec {
            enforce: true,
            allow: rules(allow),
            deny: rules(deny),
            ..NetworkSpec::default()
        }
    }

    fn disabled(network: NetworkSpec) -> NetworkSpec {
        NetworkSpec {
            enabled: Some(false),
            ..network
        }
    }

    /// Returns the config microsandbox would create a sandbox with after `add_to_builder` and the
    /// secrets.
    async fn config_with(network: &NetworkSpec, secrets: &[Secret]) -> SandboxConfig {
        let builder = Sandbox::builder("fbk-unit-network").image("alpine");
        let builder = add_to_builder(builder, network).unwrap();

        crate::secrets::add_to_builder(builder, secrets)
            .build()
            .await
            .unwrap()
    }

    /// Describes an egress rule as `<action> <destination>`.
    fn describe(rule: &Rule) -> String {
        assert_eq!(rule.direction, Direction::Egress);

        let action = match rule.action {
            Action::Allow => "allow",
            Action::Deny => "deny",
        };
        let destination = match &rule.destination {
            Destination::Domain(domain) => format!("domain {domain}"),
            Destination::DomainSuffix(domain) => format!("suffix {domain}"),
            Destination::Cidr(network) => format!("cidr {network}"),
            other => format!("{other:?}"),
        };

        format!("{action} {destination}")
    }

    #[test]
    fn rules_label_lists_enforced_rules() {
        assert_eq!(
            rules_label(&enforced(&["github.com", "*.npmjs.org"], &["10.0.0.0/8"])),
            "allow=github.com,*.npmjs.org;deny=10.0.0.0/8"
        );
        assert_eq!(rules_label(&enforced(&[], &[])), "allow=;deny=");
    }

    #[test]
    fn rules_label_ignores_rules_that_are_not_enforced() {
        let off = NetworkSpec {
            enforce: false,
            ..enforced(&["github.com"], &[])
        };

        assert_eq!(rules_label(&off), "off");
        assert_eq!(rules_label(&NetworkSpec::default()), "off");
    }

    #[test]
    fn rules_label_records_a_disabled_network_without_rules() {
        assert_eq!(
            rules_label(&disabled(enforced(&["github.com"], &[]))),
            "disabled"
        );
        assert_eq!(rules_label(&disabled(NetworkSpec::default())), "disabled");
    }

    #[test]
    fn rules_label_of_an_explicitly_enabled_network_matches_the_default() {
        let enabled = |network: NetworkSpec| NetworkSpec {
            enabled: Some(true),
            ..network
        };

        assert_eq!(rules_label(&enabled(NetworkSpec::default())), "off");
        assert_eq!(
            rules_label(&enabled(enforced(&["github.com"], &[]))),
            "allow=github.com;deny="
        );
    }

    #[tokio::test]
    async fn disabled_network_removes_the_device_and_ignores_the_rules() {
        let config = config_with(&disabled(enforced(&["github.com"], &[])), &[]).await;

        assert!(!config.spec.network.enabled);
    }

    #[tokio::test]
    async fn secrets_do_not_enable_a_disabled_network() {
        let secret = Secret::new("GH_TOKEN".to_string(), "abc".to_string(), vec![]).unwrap();

        let config = config_with(&disabled(NetworkSpec::default()), &[secret]).await;

        assert!(!config.spec.network.enabled);
    }

    #[tokio::test]
    async fn enabled_network_keeps_the_device() {
        let config = config_with(&enforced(&["github.com"], &[]), &[]).await;

        assert!(config.spec.network.enabled);
    }

    #[test]
    fn policy_denies_egress_by_default() {
        let policy = policy(&enforced(&[], &[])).unwrap();

        assert_eq!(policy.default_egress, Action::Deny);
    }

    #[test]
    fn policy_allows_dns_last() {
        let policy = policy(&enforced(&["github.com"], &["gist.github.com"])).unwrap();

        assert_eq!(policy.rules.len(), 3);
        assert_eq!(
            format!("{:?}", policy.rules[2]),
            format!("{:?}", Rule::allow_dns())
        );
    }

    #[test]
    fn policy_puts_deny_rules_before_allow_rules() {
        let policy = policy(&enforced(
            &["*.github.com", "10.0.0.0/8"],
            &["gist.github.com", "10.1.2.3"],
        ))
        .unwrap();

        let (_dns, rules) = policy.rules.split_last().unwrap();
        let described: Vec<String> = rules.iter().map(describe).collect();

        assert_eq!(
            described,
            [
                "deny domain gist.github.com",
                "deny cidr 10.1.2.3/32",
                "allow suffix github.com",
                "allow cidr 10.0.0.0/8",
            ]
        );
    }

    #[test]
    fn policy_maps_each_rule_form_to_its_destination() {
        let policy = policy(&enforced(
            &[
                "github.com",
                "*.githubusercontent.com",
                "140.82.112.4",
                "2001:db8::1",
                "192.168.10.0/24",
            ],
            &[],
        ))
        .unwrap();

        let (_dns, rules) = policy.rules.split_last().unwrap();
        let described: Vec<String> = rules.iter().map(describe).collect();

        assert_eq!(
            described,
            [
                "allow domain github.com",
                "allow suffix githubusercontent.com",
                "allow cidr 140.82.112.4/32",
                "allow cidr 2001:db8::1/128",
                "allow cidr 192.168.10.0/24",
            ]
        );
    }

    #[test]
    fn deny_message_names_the_host_and_the_command() {
        assert_eq!(
            DENY_MESSAGE.replace("{host}", "example.org"),
            "firebrick blocked the connection to example.org: the network policy of this \
             sandbox doesn't allow it. To allow it, run `fbk network allow example.org` on the \
             host, outside the sandbox."
        );
    }
}
