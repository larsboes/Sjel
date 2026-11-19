# Transit reliability

Validate existing results before collection and model expansion. Watch sets and personal routes stay on owner-controlled machines. The [reliability design](../architecture/transit-reliability.md) owns the proposed model.

<a id="5-work-packages"></a>

## Open

- [Measure corridor source coverage](../tasks/transit-observable.md).
- [Record realised rides](../tasks/transit-realised.md).
- [Key reliability by line](../tasks/transit-line-cells.md).
- [Show disruption remarks](../tasks/transit-remarks.md).
- [Score missed-transfer waiting](../tasks/transit-headway.md).
- [Collect corridor observations](../tasks/transit-collector-mac.md).
- [Collect with the Mac closed](../tasks/transit-collector-host.md).
- [Compare corridor reliability](../tasks/transit-report.md).
- [Advise before departure](../tasks/transit-predeparture.md).
- [Review history-derived routes](../tasks/transit-preferences.md).
- [Plan stays between travel stages](../tasks/transit-stopovers.md).
- [Evaluate broader reliability models](../tasks/transit-model-research.md).

## Order

Source coverage and realised rides precede collection/model expansion. Hosted collection waits for
a selected owner-controlled host; no datacenter receives a location routine. Phone advice follows
Priority 4. Proposed broader models remain unvalidated until compared against observed outcomes.
