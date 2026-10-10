//! The attention queue: what waits for the author. Permissions come first,
//! then finished turns, each group in arrival order, so `Tab` reaches every
//! item.

use std::collections::VecDeque;
use std::fmt;

use crate::agent::AgentId;
use crate::grammar::Name;
use crate::stream::PermissionRequest;

/// An attention item id, unique for the life of the core, so an answer
/// reaches only the request the author saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(u64);

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A permission request. Its agent waits until the author answers it.
#[derive(Debug)]
pub struct Permission {
    pub agent: AgentId,
    pub target: Name,
    pub request: PermissionRequest,
    /// The request block, shown in full before the question.
    pub block: String,
    /// Rule 1 found a setup path.
    pub setup: bool,
}

/// A turn that ended. It keeps the turn's last text, so a question that
/// scrolled away shows again at bring-up.
#[derive(Debug)]
pub struct FinishedTurn {
    pub agent: AgentId,
    pub target: Name,
    pub last_text: String,
}

#[derive(Debug)]
pub enum Item {
    Permission(Permission),
    FinishedTurn(FinishedTurn),
}

#[derive(Debug, Default)]
pub struct Queue {
    permissions: VecDeque<(ItemId, Permission)>,
    turns: VecDeque<(ItemId, FinishedTurn)>,
    next: u64,
}

impl Queue {
    pub fn push_permission(&mut self, permission: Permission) -> ItemId {
        let id = self.next_id();
        self.permissions.push_back((id, permission));
        id
    }

    pub fn push_turn(&mut self, turn: FinishedTurn) -> ItemId {
        let id = self.next_id();
        self.turns.push_back((id, turn));
        id
    }

    /// The item `Tab` brings up.
    pub fn first(&self) -> Option<ItemId> {
        self.permissions
            .front()
            .map(|(id, _)| *id)
            .or_else(|| self.turns.front().map(|(id, _)| *id))
    }

    /// The item a mention-only line brings up: the target's permission
    /// before its finished turn.
    pub fn for_target(&self, target: &Name) -> Option<ItemId> {
        self.permission_for(target).or_else(|| {
            self.turns
                .iter()
                .find(|(_, turn)| turn.target == *target)
                .map(|(id, _)| *id)
        })
    }

    pub fn permission_for(&self, target: &Name) -> Option<ItemId> {
        self.permissions
            .iter()
            .find(|(_, permission)| permission.target == *target)
            .map(|(id, _)| *id)
    }

    pub fn permission(&self, id: ItemId) -> Option<&Permission> {
        self.permissions
            .iter()
            .find(|(item, _)| *item == id)
            .map(|(_, permission)| permission)
    }

    /// Moves the item to the end of its group. `false` when it no longer
    /// waits.
    pub fn later(&mut self, id: ItemId) -> bool {
        move_to_back(&mut self.permissions, id) || move_to_back(&mut self.turns, id)
    }

    pub fn remove(&mut self, id: ItemId) -> Option<Item> {
        take(&mut self.permissions, |item, _| item == id)
            .map(|(_, permission)| Item::Permission(permission))
            .or_else(|| {
                take(&mut self.turns, |item, _| item == id)
                    .map(|(_, turn)| Item::FinishedTurn(turn))
            })
    }

    /// Removes and returns the agent's waiting permissions, oldest first.
    pub fn take_permissions_of(&mut self, agent: AgentId) -> Vec<(ItemId, Permission)> {
        let mut taken = Vec::new();
        while let Some(entry) = take(&mut self.permissions, |_, permission| {
            permission.agent == agent
        }) {
            taken.push(entry);
        }
        taken
    }

    /// Removes the target's finished turn, which a new request to it
    /// replaces.
    pub fn remove_turn_of(&mut self, target: &Name) {
        self.turns.retain(|(_, turn)| turn.target != *target);
    }

    /// Removes every item of an agent that has ended.
    pub fn remove_agent(&mut self, agent: AgentId) {
        self.permissions
            .retain(|(_, permission)| permission.agent != agent);
        self.turns.retain(|(_, turn)| turn.agent != agent);
    }

    pub fn len(&self) -> usize {
        self.permissions.len() + self.turns.len()
    }

    const fn next_id(&mut self) -> ItemId {
        let id = ItemId(self.next);
        self.next += 1;
        id
    }
}

fn take<T>(
    group: &mut VecDeque<(ItemId, T)>,
    matches: impl Fn(ItemId, &T) -> bool,
) -> Option<(ItemId, T)> {
    let index = group.iter().position(|(id, item)| matches(*id, item))?;
    group.remove(index)
}

fn move_to_back<T>(group: &mut VecDeque<(ItemId, T)>, id: ItemId) -> bool {
    let Some(entry) = take(group, |item, _| item == id) else {
        return false;
    };
    group.push_back(entry);
    true
}
