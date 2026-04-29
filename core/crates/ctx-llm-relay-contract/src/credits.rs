use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditGrantSource {
    PromoTrial,
    SubscriptionIncluded,
    EnterpriseCommit,
    PrepaidTopUp,
}

impl CreditGrantSource {
    fn priority(&self) -> u8 {
        match self {
            CreditGrantSource::PromoTrial => 0,
            CreditGrantSource::SubscriptionIncluded => 1,
            CreditGrantSource::EnterpriseCommit => 2,
            CreditGrantSource::PrepaidTopUp => 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditGrant {
    pub grant_id: String,
    pub billing_subject_id: String,
    pub source: CreditGrantSource,
    pub total_cents: u64,
    pub remaining_cents: u64,
    pub issued_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl CreditGrant {
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.remaining_cents > 0
            && self
                .expires_at
                .map(|expires_at| expires_at > now)
                .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditAllocation {
    pub grant_id: String,
    pub source: CreditGrantSource,
    pub reserved_cents: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditReservation {
    pub requested_cents: u64,
    pub allocations: Vec<CreditAllocation>,
}

impl CreditReservation {
    pub fn total_reserved_cents(&self) -> u64 {
        self.allocations
            .iter()
            .map(|allocation| allocation.reserved_cents)
            .sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditAllocationSettlement {
    pub grant_id: String,
    pub source: CreditGrantSource,
    pub reserved_cents: u64,
    pub finalized_cents: u64,
    pub released_cents: u64,
    pub released_spendable_cents: u64,
    pub released_expired_cents: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CreditAllocationError {
    #[error("requested reservation must be greater than zero")]
    ZeroRequestedCents,
    #[error("insufficient credits for reservation: requested {requested_cents}, available {available_cents}")]
    InsufficientCredits {
        requested_cents: u64,
        available_cents: u64,
    },
    #[error("actual finalized cents {actual_cents} exceeds reserved cents {reserved_cents}")]
    ActualExceedsReserved {
        actual_cents: u64,
        reserved_cents: u64,
    },
}

pub fn allocate_credit_reservation(
    grants: &[CreditGrant],
    requested_cents: u64,
    now: DateTime<Utc>,
) -> Result<CreditReservation, CreditAllocationError> {
    if requested_cents == 0 {
        return Err(CreditAllocationError::ZeroRequestedCents);
    }

    let mut candidates: Vec<&CreditGrant> = grants
        .iter()
        .filter(|grant| grant.is_active_at(now))
        .collect();
    candidates.sort_by(|left, right| {
        left.source
            .priority()
            .cmp(&right.source.priority())
            .then_with(|| expiry_sort_key(left.expires_at).cmp(&expiry_sort_key(right.expires_at)))
            .then_with(|| left.grant_id.cmp(&right.grant_id))
    });

    let available_cents: u64 = candidates.iter().map(|grant| grant.remaining_cents).sum();
    if available_cents < requested_cents {
        return Err(CreditAllocationError::InsufficientCredits {
            requested_cents,
            available_cents,
        });
    }

    let mut remaining = requested_cents;
    let mut allocations = Vec::new();

    for grant in candidates {
        if remaining == 0 {
            break;
        }
        let reserved_cents = remaining.min(grant.remaining_cents);
        if reserved_cents == 0 {
            continue;
        }
        allocations.push(CreditAllocation {
            grant_id: grant.grant_id.clone(),
            source: grant.source.clone(),
            reserved_cents,
            grant_expires_at: grant.expires_at,
        });
        remaining -= reserved_cents;
    }

    Ok(CreditReservation {
        requested_cents,
        allocations,
    })
}

pub fn finalize_credit_reservation(
    reservation: &CreditReservation,
    actual_cents: u64,
    now: DateTime<Utc>,
) -> Result<Vec<CreditAllocationSettlement>, CreditAllocationError> {
    settle_credit_allocations(&reservation.allocations, actual_cents, now)
}

pub fn release_credit_reservation(
    reservation: &CreditReservation,
    now: DateTime<Utc>,
) -> Vec<CreditAllocationSettlement> {
    reservation
        .allocations
        .iter()
        .map(|allocation| {
            let released_cents = allocation.reserved_cents;
            let released_spendable_cents = if grant_still_active(allocation.grant_expires_at, now) {
                released_cents
            } else {
                0
            };
            CreditAllocationSettlement {
                grant_id: allocation.grant_id.clone(),
                source: allocation.source.clone(),
                reserved_cents: allocation.reserved_cents,
                finalized_cents: 0,
                released_cents,
                released_spendable_cents,
                released_expired_cents: released_cents - released_spendable_cents,
            }
        })
        .collect()
}

fn settle_credit_allocations(
    allocations: &[CreditAllocation],
    actual_cents: u64,
    now: DateTime<Utc>,
) -> Result<Vec<CreditAllocationSettlement>, CreditAllocationError> {
    let reserved_cents: u64 = allocations
        .iter()
        .map(|allocation| allocation.reserved_cents)
        .sum();
    if actual_cents > reserved_cents {
        return Err(CreditAllocationError::ActualExceedsReserved {
            actual_cents,
            reserved_cents,
        });
    }

    let mut remaining_actual = actual_cents;
    let mut settlements = Vec::with_capacity(allocations.len());

    for allocation in allocations {
        let finalized_cents = remaining_actual.min(allocation.reserved_cents);
        remaining_actual -= finalized_cents;
        let released_cents = allocation.reserved_cents - finalized_cents;
        let released_spendable_cents = if grant_still_active(allocation.grant_expires_at, now) {
            released_cents
        } else {
            0
        };
        settlements.push(CreditAllocationSettlement {
            grant_id: allocation.grant_id.clone(),
            source: allocation.source.clone(),
            reserved_cents: allocation.reserved_cents,
            finalized_cents,
            released_cents,
            released_spendable_cents,
            released_expired_cents: released_cents - released_spendable_cents,
        });
    }

    Ok(settlements)
}

fn expiry_sort_key(expires_at: Option<DateTime<Utc>>) -> (u8, Option<DateTime<Utc>>) {
    (u8::from(expires_at.is_none()), expires_at)
}

fn grant_still_active(expires_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    expires_at.map(|value| value > now).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{
        allocate_credit_reservation, finalize_credit_reservation, CreditGrant, CreditGrantSource,
        CreditReservation,
    };

    fn grant(
        grant_id: &str,
        source: CreditGrantSource,
        remaining_cents: u64,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> CreditGrant {
        let issued_at = Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap();
        CreditGrant {
            grant_id: grant_id.to_string(),
            billing_subject_id: "bill_1".to_string(),
            source,
            total_cents: remaining_cents,
            remaining_cents,
            issued_at,
            expires_at,
        }
    }

    #[test]
    fn credit_reservations_follow_deterministic_priority_order() {
        let now = Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
        let grants = vec![
            grant("topup_never", CreditGrantSource::PrepaidTopUp, 30, None),
            grant(
                "sub_later",
                CreditGrantSource::SubscriptionIncluded,
                20,
                Some(now + Duration::days(30)),
            ),
            grant(
                "promo_soon",
                CreditGrantSource::PromoTrial,
                10,
                Some(now + Duration::days(1)),
            ),
            grant(
                "enterprise_mid",
                CreditGrantSource::EnterpriseCommit,
                15,
                Some(now + Duration::days(10)),
            ),
            grant(
                "topup_soon",
                CreditGrantSource::PrepaidTopUp,
                25,
                Some(now + Duration::days(3)),
            ),
            grant(
                "promo_later",
                CreditGrantSource::PromoTrial,
                10,
                Some(now + Duration::days(7)),
            ),
        ];

        let reservation = allocate_credit_reservation(&grants, 90, now).unwrap();
        let ordered_ids: Vec<&str> = reservation
            .allocations
            .iter()
            .map(|allocation| allocation.grant_id.as_str())
            .collect();

        assert_eq!(
            ordered_ids,
            vec![
                "promo_soon",
                "promo_later",
                "sub_later",
                "enterprise_mid",
                "topup_soon",
                "topup_never"
            ]
        );
        assert_eq!(reservation.total_reserved_cents(), 90);
    }

    #[test]
    fn credit_finalization_releases_unused_value_back_or_to_expired() {
        let reservation = CreditReservation {
            requested_cents: 90,
            allocations: vec![
                super::CreditAllocation {
                    grant_id: "promo_soon".to_string(),
                    source: CreditGrantSource::PromoTrial,
                    reserved_cents: 50,
                    grant_expires_at: Some(Utc.with_ymd_and_hms(2026, 4, 28, 12, 1, 0).unwrap()),
                },
                super::CreditAllocation {
                    grant_id: "topup_never".to_string(),
                    source: CreditGrantSource::PrepaidTopUp,
                    reserved_cents: 40,
                    grant_expires_at: None,
                },
            ],
        };
        let now = Utc.with_ymd_and_hms(2026, 4, 28, 12, 2, 0).unwrap();

        let settlements = finalize_credit_reservation(&reservation, 40, now).unwrap();

        assert_eq!(settlements[0].finalized_cents, 40);
        assert_eq!(settlements[0].released_cents, 10);
        assert_eq!(settlements[0].released_spendable_cents, 0);
        assert_eq!(settlements[0].released_expired_cents, 10);

        assert_eq!(settlements[1].finalized_cents, 0);
        assert_eq!(settlements[1].released_cents, 40);
        assert_eq!(settlements[1].released_spendable_cents, 40);
        assert_eq!(settlements[1].released_expired_cents, 0);
    }
}
