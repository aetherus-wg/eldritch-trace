//! Evaluate - Rule evaluation
//!
//! This module provides functionality for evaluating rules
//! against Ledger hierarchy of events that encodes the chain of events

use aetherus_events::{Ledger, Uid};
use anyhow::Result;

use crate::Check;
use crate::model::{Match, Predicate, Rule, RuleCond, Seq};

impl Rule {
    /// Evaluate this rule against a ledger.
    ///
    /// Returns a list of UIDs that match the rule.
    pub fn evaluate(&self, ledger: &Ledger) -> Result<Vec<Uid>> {
        find_uids_with_rule(ledger, self)
    }
}

/// Find UIDs that match a rule.
///
/// Iterates through the ledger and finds all events
/// that satisfy the given rule.
///
/// # Arguments
///
/// * `ledger` - The event ledger to search
/// * `rule` - The rule to evaluate
///
/// # Returns
///
/// A list of matching UIDs
pub fn find_uids_with_rule(ledger: &Ledger, rule: &Rule) -> Result<Vec<Uid>> {
    let mut found_uids: Vec<Uid> = Vec::new();

    #[derive(Clone, Debug)]
    pub enum CondTraverse<'a> {
        Pattern {
            pred: Predicate,
            event_match: &'a Match,
            cnt: usize,
        },
        Seq {
            seq_idx: usize,
            seq: &'a Seq,
            cnt: usize,
        },
    }
    #[derive(Clone)]
    pub struct RuleTraverse<'a> {
        pub uid: Uid,
        pub cond_idx: usize,
        pub conds: Vec<CondTraverse<'a>>,
    }

    impl RuleTraverse<'_> {
        pub fn satfisfied(&self) -> bool {
            if self.cond_idx != 0 {
                return false;
            }
            for cond in self.conds.iter() {
                match cond {
                    CondTraverse::Pattern{pred, event_match:_, cnt} => {
                        match pred {
                            Predicate::Unit      => return false,
                            Predicate::Not       => (),
                            Predicate::Repeat(r) => {
                                if !r.check(*cnt) {
                                    return false;
                                }
                            }
                        }
                    }
                    CondTraverse::Seq{seq_idx, seq, cnt:_} => {
                        if *seq_idx < seq.0.len() {
                            return false;
                        }
                    }
                }
            }
            true
        }
    }

    let mut stack: Vec<RuleTraverse> = Vec::new();

    // Initial stack entries
    let mut conds = Vec::new();
    for cond in rule.0.iter().rev() {
        match cond {
            RuleCond::Pattern(pred, event_match) => conds.push(CondTraverse::Pattern {
                pred: pred.clone(),
                event_match,
                cnt: 0,
            }),
            RuleCond::Seq(seq) => {
                conds.push(CondTraverse::Seq {
                    seq_idx: 0,
                    seq,
                    cnt: 0,
                });
            }
        }
    }
    for &uid in ledger.get_start_events() {
        stack.push(RuleTraverse {
            uid,
            cond_idx: 0,
            conds: conds.clone(),
        });
    }

    // Ledger traversal loop
    // TODO: Entries in the stack can be evaluated in parallel
    // and querry of the Ledger should not require a lock, since the Ledger is immutable
    while !stack.is_empty() {
        let mut rule = stack.pop().unwrap();

        //println!("Evaluating rule at UID: {:?}, condition index: {}, conditions: {:?}", rule.uid, rule.cond_idx, rule.conds);

        let mut pass = true;
        let mut remove = false;
        let mut bifurcate = false;

        let mut recheck = true;
        while recheck {
            let cond = &mut rule.conds[rule.cond_idx];
            recheck = false;
            match cond {
                CondTraverse::Pattern { pred, event_match, cnt } => {
                    let event_check = event_match.check(rule.uid.event);
                    match pred {
                        Predicate::Unit => {
                            if event_check {
                                // Condition satified and ready to remove
                                remove = true;
                            }
                        }
                        Predicate::Not => {
                            if event_check {
                                pass = false;
                            }
                        }
                        Predicate::Repeat(r) => {
                            if event_check {
                                *cnt = *cnt + 1;
                                if r.min() > *cnt {
                                    // Just increment count and stay on the same sequence condition until reaching the minimum required repetitions
                                } else {
                                    if let Some(upper) = r.max() {
                                        if *cnt > upper {
                                            pass = false;
                                        }
                                    } else {
                                        remove = true;
                                    }
                                }
                            }
                        }
                    }
                }
                CondTraverse::Seq { seq_idx, seq, cnt } => {
                    if *seq_idx >= seq.0.len() {
                        pass = false;
                        break;
                    }

                    let (pred, pattern_match) = &seq.0[*seq_idx];
                    let event_check = pattern_match.check(rule.uid.event);
                    match pred {
                        // Indexes of sequence conditions that are currently in neg check (predicate=!),
                        // to be evaluated on the same UID without advancing
                        Predicate::Not => {
                            if event_check {
                                pass = false;
                            } else {
                                recheck = true;
                                *cond = CondTraverse::Seq {seq_idx: *seq_idx + 1, cnt: 0, seq};
                            }
                        }
                        Predicate::Unit => {
                            if event_check {
                                *cond = CondTraverse::Seq {seq_idx: *seq_idx + 1, cnt: 0, seq};
                            } else {
                                pass = false;
                            }
                        }
                        Predicate::Repeat(r) => {
                            if event_check {
                                *cnt = *cnt + 1;
                                if r.min() > *cnt {
                                    // Just increment count and stay on the same sequence condition until reaching the minimum required repetitions
                                } else {
                                    if let Some(upper) = r.max() {
                                        assert!(
                                            r.min() <= upper,
                                            "Invalid repetition predicate with min > max in sequence condition"
                                        );
                                        if *cnt >= upper {
                                            // Satisfied, move on
                                            *cond = CondTraverse::Seq {seq_idx: *seq_idx + 1, cnt: 0, seq};
                                        } else {
                                            // Satisfied, but we might allow this pattern to
                                            // consume more events
                                            bifurcate = true;
                                        }
                                    } else {
                                        bifurcate = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        assert!(
            !remove || !bifurcate,
            "Bifurcate(Seq) and Remove(Pattern) are mutually exclusive flags"
        );

        if pass {
            let cond_idx = rule.cond_idx;
            let next_uids = ledger.get_next(&rule.uid);

            if remove {
                rule.conds.remove(rule.cond_idx);
                if rule.conds.len() == cond_idx {
                    rule.cond_idx = 0;
                }
            } else {
                if rule.cond_idx + 1 == rule.conds.len() {
                    rule.cond_idx = 0;
                } else {
                    rule.cond_idx = rule.cond_idx + 1;
                }
            }

            if bifurcate {
                let mut bifurcated_rule = rule.clone();
                if let CondTraverse::Seq {seq_idx, seq, cnt:_} = bifurcated_rule.conds[cond_idx]
                {
                    bifurcated_rule.conds[cond_idx] = CondTraverse::Seq {
                        seq_idx: seq_idx + 1,
                        cnt: 0,
                        seq,
                    };
                } else {
                    return Err(anyhow::anyhow!(
                        "Expected a sequence condition for bifurcation"
                    ));
                }

                if rule.cond_idx == 0 {
                    for &next_uid in next_uids.iter() {
                        stack.push(RuleTraverse {
                            uid: next_uid,
                            cond_idx: bifurcated_rule.cond_idx,
                            conds: bifurcated_rule.conds.clone(),
                        });
                    }
                    if next_uids.is_empty() && rule.satfisfied() {
                        //println!("Found a match for rule with UID: {:?}", rule.uid);
                        if !found_uids.contains(&rule.uid) {
                            found_uids.push(rule.uid);
                        }
                    }
                } else {
                    stack.push(bifurcated_rule);
                }
            }

            if rule.cond_idx == 0 {
                for &next_uid in next_uids.iter() {
                    stack.push(RuleTraverse {
                        uid: next_uid,
                        cond_idx: rule.cond_idx,
                        conds: rule.conds.clone(),
                    });
                }
                if next_uids.is_empty() && rule.satfisfied() {
                    //println!("Found a match for rule with UID: {:?}", rule.uid);
                    if !found_uids.contains(&rule.uid) {
                        found_uids.push(rule.uid);
                    }
                }
            } else {
                stack.push(rule);
            }
        }
    }

    Ok(found_uids)
}
