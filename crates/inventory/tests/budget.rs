use sjel_inventory::budget::{MonthlyBalance, months_to_pay};

#[test]
fn only_a_positive_balance_can_fund_wishes() {
    let mut balance = MonthlyBalance {
        median_cents: 25_000,
        months: 1,
        from: "2025-05".into(),
        to: "2025-05".into(),
    };
    assert_eq!(months_to_pay(100_000, &balance), Some(4.0));
    balance.median_cents = 0;
    assert_eq!(months_to_pay(100_000, &balance), None);
    balance.median_cents = -30_000;
    assert_eq!(months_to_pay(100_000, &balance), None);
}
