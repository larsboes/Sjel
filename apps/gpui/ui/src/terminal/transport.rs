use std::cell::RefCell;

use futures::{channel::mpsc, StreamExt};
use gpui::{AppContext, Context, Entity, Task};

use super::TerminalView;
use crate::ely::terminal::{RemoteTransport, Terminal};

enum Request {
    Input(Vec<u8>),
    Resize(usize, usize),
}

/// Queues callbacks so a child never reborrows its host during rendering or input dispatch.
pub(super) fn connect(
    dimensions: (usize, usize),
    cx: &mut Context<TerminalView>,
) -> (Entity<Terminal>, Task<()>) {
    let (sender, mut receiver) = mpsc::channel(256);
    let resize = RefCell::new(sender.clone());
    let input = RefCell::new(sender);
    let terminal = cx.new(|cx| {
        Terminal::remote(
            dimensions.0,
            dimensions.1,
            RemoteTransport::new(
                move |bytes| {
                    input
                        .borrow_mut()
                        .try_send(Request::Input(bytes.to_vec()))
                        .map_err(Into::into)
                },
                move |columns, lines| {
                    resize
                        .borrow_mut()
                        .try_send(Request::Resize(columns, lines))
                        .map_err(Into::into)
                },
            ),
            cx,
        )
    });
    let task = cx.spawn(async move |weak, cx| {
        while let Some(request) = receiver.next().await {
            if weak
                .update(cx, |view, cx| match request {
                    Request::Input(bytes) => view.queue_input(bytes, cx),
                    Request::Resize(columns, lines) => view.resize(columns, lines, cx),
                })
                .is_err()
            {
                return;
            }
        }
    });
    (terminal, task)
}
