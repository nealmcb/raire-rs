// Copyright 2023 Andrew Conway.
// Based on software (c) Michelle Blom in C++ https://github.com/michelleblom/audit-irv-cp/tree/raire-branch
// documented in https://arxiv.org/pdf/1903.08804.pdf
//
// This file is part of raire-rs.
// raire-rs is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
// raire-rs is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU Affero General Public License for more details.
// You should have received a copy of the GNU Affero General Public License along with ConcreteSTV.  If not, see <https://www.gnu.org/licenses/>.

use std::cmp::Ordering;
use crate::assertions::{Assertion, AssertionAndDifficulty, EffectOfAssertionOnEliminationOrderSuffix};
use crate::irv::CandidateIndex;
use crate::RaireError;

/// Produce a tree of reverse-elimination-order descending down until either
/// * At least one assertion prunes all subsequent orders
/// * No assertions prune any subsequent order
///
/// One can optionally ask for an extended tree, which extends pruned nodes one extra step
/// if each of their children is also pruned. This is useful for finding redundant assertions
/// that can be removed, at the cost of making the frontier larger.
pub struct TreeNodeShowingWhatAssertionsPrunedIt {
    pub candidate_being_eliminated_at_this_node: CandidateIndex, // The candidate eliminated at this step.
    pub pruning_assertions : Vec<usize>, // if any assertions prune it, their index in the main assertion list.
    pub children : Vec<TreeNodeShowingWhatAssertionsPrunedIt>, // its children, if any.
    pub valid : bool, // whether this node or a child thereof is not eliminated by any assertion.
}

impl TreeNodeShowingWhatAssertionsPrunedIt {
    /// Create a new tree node with a given path back to the root and candidate being eliminated.
    pub fn new (parent_elimination_order_suffix:&[CandidateIndex], candidate_being_eliminated_at_this_node:CandidateIndex, relevant_assertions:&[usize],all_assertions:&[Assertion],num_candidates:u32,consider_children_of_eliminated_nodes:bool) -> Self {
        let mut elimination_order_suffix=vec![candidate_being_eliminated_at_this_node]; // elimination order including this node
        elimination_order_suffix.extend_from_slice(parent_elimination_order_suffix);
        let mut pruning_assertions : Vec<usize> = vec![];
        let mut still_relevant_assertions : Vec<usize> = vec![];
        for &assertion_index in relevant_assertions {
            match all_assertions[assertion_index].ok_elimination_order_suffix(&elimination_order_suffix) {
                EffectOfAssertionOnEliminationOrderSuffix::Contradiction => { pruning_assertions.push(assertion_index); }
                EffectOfAssertionOnEliminationOrderSuffix::Ok => {} // can ignore
                EffectOfAssertionOnEliminationOrderSuffix::NeedsMoreDetail => { still_relevant_assertions.push(assertion_index); }
            }
        }
        let mut children : Vec<Self> = vec![];
        let mut valid : bool = pruning_assertions.is_empty() && still_relevant_assertions.is_empty();
        if (pruning_assertions.is_empty()||consider_children_of_eliminated_nodes) && !still_relevant_assertions.is_empty() {
            for candidate in 0..num_candidates {
                let candidate = CandidateIndex(candidate);
                if !elimination_order_suffix.contains(&candidate) { // could make more efficient by using binary search,
                    let child = TreeNodeShowingWhatAssertionsPrunedIt::new(&elimination_order_suffix,candidate,&still_relevant_assertions,all_assertions,num_candidates,consider_children_of_eliminated_nodes&&pruning_assertions.is_empty());
                    if child.valid { valid=true; }
                    children.push(child);
                }
            }
        }
        if consider_children_of_eliminated_nodes && !pruning_assertions.is_empty() {
            if valid { // at least one of the children was not ruled out. Going an additional step is not useful.
                children.clear();
                valid=false;
            }
        }
        TreeNodeShowingWhatAssertionsPrunedIt{candidate_being_eliminated_at_this_node,pruning_assertions,children,valid}
    }
}

/// Change the list of assertions to order them with the first removing the most undesired elimination orders,
/// the second removing the most of what is left, etc.
///
/// Assertions that don't remove anything other than from places where the winner ends will be removed.
///
/// consider_children_of_eliminated_nodes, if true, will take a little longer and possibly produce a smaller number of assertions
/// at the cost of a larger tree size for the eliminated paths tree.
pub fn order_assertions_and_remove_unnecessary(assertions:&mut Vec<AssertionAndDifficulty>,winner:CandidateIndex,num_candidates:u32,consider_children_of_eliminated_nodes:bool) -> Result<(),RaireError> {
    assertions.sort_unstable_by(|a,b|{
        // sort all NENs before NEBs,
        // sort NENs by length
        // ties - sort by winner, then loser, then continuing
        match (&a.assertion,&b.assertion) {
            (Assertion::NEN(_), Assertion::NEB(_)) => Ordering::Less,
            (Assertion::NEB(_), Assertion::NEN(_)) => Ordering::Greater,
            (Assertion::NEN(a), Assertion::NEN(b)) => {
                a.continuing.len().cmp(&b.continuing.len()).then_with(||a.winner.0.cmp(&b.winner.0).then_with(||a.loser.0.cmp(&b.loser.0)).then_with(||{
                    // compare continuing
                    for i in 0..a.continuing.len() {
                        let res = a.continuing[i].0.cmp(&b.continuing[i].0);
                        if res!=Ordering::Equal { return res}
                    }
                    Ordering::Equal
                }))
            },
            (Assertion::NEB(a), Assertion::NEB(b)) => a.winner.0.cmp(&b.winner.0).then_with(||a.loser.0.cmp(&b.loser.0)),
        }
    });
    let all_assertions : Vec<Assertion> = assertions.iter().map(|ad|ad.assertion.clone()).collect();
    let all_assertion_indices : Vec<usize> = (0..all_assertions.len()).collect();
    let mut find_used = SimplisticWorkOutWhichAssertionsAreUsed::new(assertions.len());
    let mut trees = vec![];
    for candidate in 0..num_candidates {
        let candidate = CandidateIndex(candidate);
        let tree = TreeNodeShowingWhatAssertionsPrunedIt::new(&[],candidate,&all_assertion_indices,&all_assertions,num_candidates,consider_children_of_eliminated_nodes);
        if tree.valid!= (candidate==winner) { return Err(if candidate==winner { RaireError::InternalErrorRuledOutWinner} else { RaireError::InternalErrorDidntRuleOutLoser })}
        if candidate!=winner {
            find_used.add_tree_forced(&tree);
            trees.push(tree);
        }
    }
    for tree in trees {
        find_used.add_tree_second_pass(&tree);
    }
    let mut res = vec![];
    for (index,a) in assertions.drain(..).enumerate() {
        if find_used.uses(index) { res.push(a); }
    }
    assertions.extend(res.drain(..));
    println!(" Trimmed {} assertions down to {}",all_assertion_indices.len(),assertions.len());
    Ok(())
}

/// a really simplistic method of computing which assertions are used - just use the first from each list. Benefits: fast, simple. Drawbacks: Not optimal.
struct SimplisticWorkOutWhichAssertionsAreUsed {
    assertions_used : Vec<bool>,
}

impl SimplisticWorkOutWhichAssertionsAreUsed {
    fn new(len:usize) -> Self { Self{assertions_used:vec![false;len]}}
    fn uses(&self,index:usize) -> bool { self.assertions_used[index] }
    /// Some (most) nodes have exactly one assertion. Assign these assertions, as they MUST be used.
    fn add_tree_forced(&mut self,node:&TreeNodeShowingWhatAssertionsPrunedIt) {
        if node.pruning_assertions.len()>0 {
            print!("{}",node.pruning_assertions.len());
            if node.children.is_empty() {
                if node.pruning_assertions.len()==1 { // must be used
                    self.assertions_used[node.pruning_assertions[0]]=true;
                }
            } else {
                print!("*");
            }
        } else {
            for child in &node.children {
                self.add_tree_forced(child);
            }
        }
    }
    /// See if a node is already eliminated by the assertions marked as being used.
    fn node_already_eliminated(&self,node:&TreeNodeShowingWhatAssertionsPrunedIt) -> bool {
        let directly_eliminated = node.pruning_assertions.iter().any(|&v|self.assertions_used[v]); // one of the assertions eliminates the node.
        directly_eliminated || { // check to see if all the children are eliminated
            node.children.len()!=0 && node.children.iter().all(|c|self.node_already_eliminated(c))
        }
    }
    fn add_tree_second_pass(&mut self,node:&TreeNodeShowingWhatAssertionsPrunedIt) {
        if node.pruning_assertions.len()>0 {
            print!("{}",node.pruning_assertions.len());
            if !self.node_already_eliminated(node) { // not already solved by one assertion that rules out this node.
                // none already used. Simplistically take the first one.
                self.assertions_used[node.pruning_assertions[0]]=true;
            }
        } else {
            for child in &node.children {
                self.add_tree_second_pass(child);
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use crate::assertions::{Assertion, NotEliminatedBefore, NotEliminatedNext};
    use crate::irv::CandidateIndex;
    use crate::tree_showing_what_assertions_pruned_leaves::TreeNodeShowingWhatAssertionsPrunedIt;

    fn raire_guide_assertions() -> Vec<Assertion> {
        vec![
            Assertion::NEN(NotEliminatedNext{winner:CandidateIndex(0),loser:CandidateIndex(1),continuing:vec![CandidateIndex(0),CandidateIndex(1),CandidateIndex(2),CandidateIndex(3)]}),
            Assertion::NEN(NotEliminatedNext{winner:CandidateIndex(0),loser:CandidateIndex(3),continuing:vec![CandidateIndex(0),CandidateIndex(2),CandidateIndex(3)]}),
            Assertion::NEN(NotEliminatedNext{winner:CandidateIndex(2),loser:CandidateIndex(0),continuing:vec![CandidateIndex(0),CandidateIndex(2)]}),
            Assertion::NEN(NotEliminatedNext{winner:CandidateIndex(2),loser:CandidateIndex(3),continuing:vec![CandidateIndex(0),CandidateIndex(2),CandidateIndex(3)]}),
            Assertion::NEB(NotEliminatedBefore{winner:CandidateIndex(2),loser:CandidateIndex(1)}),
            Assertion::NEN(NotEliminatedNext{winner:CandidateIndex(0),loser:CandidateIndex(3),continuing:vec![CandidateIndex(0),CandidateIndex(3)]}),
        ]
    }

    #[test]
    fn it_works() {
        let all_assertions = raire_guide_assertions();
        let relevant_assertions : Vec<usize> = (0..all_assertions.len()).collect();
        let tree0 = TreeNodeShowingWhatAssertionsPrunedIt::new(&[],CandidateIndex(0),&relevant_assertions,&all_assertions,4,false);
        let tree1 = TreeNodeShowingWhatAssertionsPrunedIt::new(&[],CandidateIndex(1),&relevant_assertions,&all_assertions,4,false);
        let tree2 = TreeNodeShowingWhatAssertionsPrunedIt::new(&[],CandidateIndex(2),&relevant_assertions,&all_assertions,4,false);
        let tree3 = TreeNodeShowingWhatAssertionsPrunedIt::new(&[],CandidateIndex(3),&relevant_assertions,&all_assertions,4,false);
        assert_eq!(false,tree0.valid);
        assert_eq!(3,tree0.children.len());
        assert_eq!(vec![4],tree0.children[0].pruning_assertions);
        assert_eq!(vec![2],tree0.children[1].pruning_assertions);
        assert_eq!(0,tree0.children[2].pruning_assertions.len());
        assert_eq!(2,tree0.children[2].children.len());
        assert_eq!(vec![4],tree0.children[2].children[0].pruning_assertions);
        assert_eq!(vec![3],tree0.children[2].children[1].pruning_assertions);
        assert_eq!(false,tree1.valid);
        assert_eq!(vec![4],tree1.pruning_assertions);
        assert_eq!(true,tree2.valid); // candidate 2 won.
        assert_eq!(false,tree3.valid);
        assert_eq!(3,tree3.children.len());
        assert_eq!(vec![5],tree3.children[0].pruning_assertions);
        assert_eq!(vec![4],tree3.children[1].pruning_assertions);
        assert_eq!(0,tree3.children[2].pruning_assertions.len());
        assert_eq!(2,tree3.children[2].children.len());
        assert_eq!(vec![1],tree3.children[2].children[0].pruning_assertions);
        assert_eq!(0,tree3.children[2].children[1].pruning_assertions.len());
        assert_eq!(vec![0],tree3.children[2].children[1].children[0].pruning_assertions);
    }
}
