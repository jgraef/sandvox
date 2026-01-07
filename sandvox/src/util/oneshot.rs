use std::sync::mpsc;

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (sender, receiver) = mpsc::sync_channel(0);
    (Sender { sender }, Receiver { receiver })
}

#[derive(Debug)]
pub struct Sender<T> {
    sender: mpsc::SyncSender<T>,
}

impl<T> Sender<T> {
    pub fn send(self, value: T) -> Result<(), SendError<T>> {
        self.sender
            .send(value)
            .map_err(|error| SendError { value: error.0 })
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    receiver: mpsc::Receiver<T>,
}

impl<T> Receiver<T> {
    pub fn receive(self) -> Result<T, ReceiveError> {
        self.receiver.recv().map_err(|mpsc::RecvError| ReceiveError)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SendError<T> {
    pub value: T,
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("oneshot channel closed")]
pub struct ReceiveError;
