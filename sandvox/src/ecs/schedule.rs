use bevy_ecs::schedule::ScheduleLabel;

#[derive(Clone, Debug, Hash, Eq, PartialEq, ScheduleLabel)]
pub struct Startup;

#[derive(Clone, Debug, Hash, Eq, PartialEq, ScheduleLabel)]
pub struct PostStartup;

#[derive(Clone, Debug, Hash, Eq, PartialEq, ScheduleLabel)]
pub struct PreUpdate;

#[derive(Clone, Debug, Hash, Eq, PartialEq, ScheduleLabel)]
pub struct Update;

#[derive(Clone, Debug, Hash, Eq, PartialEq, ScheduleLabel)]
pub struct PostUpdate;

#[derive(Clone, Debug, Hash, Eq, PartialEq, ScheduleLabel)]
pub struct Render;
