use super::*;
use skate_core::graph::activation::Child;
use skate_data::state_graph::binding::{
    Expression as BoundExpression, Operation, State as BoundState, Transition as BoundTransition,
};

fn binding() -> Binding {
    Binding {
        states: vec![BoundState {
            element: 0,
            name: "root".into(),
            parent: None,
            children: Vec::new(),
            behaviors: vec![0],
            transitions: vec![0],
            expression: Some(0),
            enabled: 1,
            active: 1,
            interruptibility: 2,
            interrupt_ancestor: Some(0),
        }],
        transitions: vec![BoundTransition {
            element: 1,
            owner: 0,
            target: Some(0),
            enabled: 1,
            priority: 3,
            expression: Some(0),
            hooks: vec![2],
        }],
        expressions: vec![BoundExpression {
            element: 2,
            enabled: 1,
            operator: 1,
            children: vec![Node::Operation(1)],
        }],
        operations: vec![
            Operation {
                element: 3,
                parent: Node::State(0),
                kind: OperationKind::Behavior,
                name: "behavior".into(),
                enabled: 1,
                condition_mask: None,
                parameters: vec![6],
            },
            Operation {
                element: 4,
                parent: Node::Expression(0),
                kind: OperationKind::Condition,
                name: "condition".into(),
                enabled: 1,
                condition_mask: Some(3),
                parameters: vec![7, 8],
            },
            Operation {
                element: 5,
                parent: Node::Transition(0),
                kind: OperationKind::Hook,
                name: "hook".into(),
                enabled: 1,
                condition_mask: None,
                parameters: Vec::new(),
            },
        ],
        root: 0,
    }
}

#[test]
fn preserves_topology_and_authored_operation_identity() {
    let compiled = CompiledGraph::from_binding(&binding()).unwrap();

    assert_eq!(compiled.program.root, 0);
    assert_eq!(compiled.program.topology.states[0].interruptibility, 2);
    assert_eq!(compiled.program.topology.transitions[0].target, 0);
    assert_eq!(compiled.program.topology.transitions[0].priority, 3);
    assert_eq!(compiled.program.activation.state_expressions, [Some(0)]);
    assert_eq!(
        compiled.program.activation.transition_expressions,
        [Some(0)]
    );
    assert_eq!(
        compiled.program.activation.expressions[0].children,
        [Child::Condition(0)]
    );
    assert_eq!(compiled.program.activation.conditions[0].mask, 3);
    assert_eq!(compiled.program.behaviors[0].owner, 0);
    assert_eq!(compiled.program.state_behaviors, [vec![0]]);
    assert_eq!(compiled.program.transition_hooks, [vec![0]]);
    assert_eq!(compiled.operations.behaviors, [0]);
    assert_eq!(compiled.operations.conditions, [1]);
    assert_eq!(compiled.operations.hooks, [2]);
}

#[test]
fn refuses_an_unresolved_transition_instead_of_choosing_a_fallback() {
    let mut source = binding();
    source.transitions[0].target = None;

    assert_eq!(
        CompiledGraph::from_binding(&source).unwrap_err(),
        GraphCompileError::UnresolvedTransitionTarget { transition: 0 }
    );
}
