pub use crate::arch::process::Context;
use core::mem;
use xous::{TID, MemoryAddress, MemoryRange, MemorySize, Message, PID, SID};

pub struct SenderID {
    pub sidx: usize,
    pub tidx: usize,
}

impl SenderID {
    pub fn from_usize(src: usize) -> Result<SenderID, xous::Error> {
        Ok(SenderID {
            sidx: src >> 16,
            tidx: src & 0xffff,
        })
    }
}

pub enum WaitingMessage {
    /// There is no waiting message.
    None,

    /// The memory was borrowed and should be returned to the given process.
    BorrowedMemory(PID, TID, MemoryAddress, MemoryAddress, MemorySize),

    /// The memory was moved, and so shouldn't be returned.
    MovedMemory,

    /// This memory should be returned to the system.
    ForgetMemory(MemoryRange),
}

/// Internal representation of a queued message for a server. This should be
/// exactly 8 words / 32 bytes, yielding 128 queued messages per server
#[repr(usize)]
#[derive(PartialEq, Debug)]
enum QueuedMessage {
    Empty,
    ScalarMessage(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* sender base address */
        usize, /* id */
        usize, /* arg1 */
        usize, /* arg2 */
        usize, /* arg3 */
        usize, /* arg4 */
    ),
    MemoryMessageSend(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* reserved */
        usize, /* id */
        usize, /* buf */
        usize, /* buf_size */
        usize, /* offset */
        usize, /* valid */
    ),
    MemoryMessageROLend(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* sender base address */
        usize, /* id */
        usize, /* buf */
        usize, /* buf_size */
        usize, /* offset */
        usize, /* valid */
    ),
    MemoryMessageRWLend(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* sender base address */
        usize, /* id */
        usize, /* buf */
        usize, /* buf_size */
        usize, /* offset */
        usize, /* valid */
    ),

    /// The process lending this memory terminated before
    /// we could receive the message.
    MemoryMessageROLendTerminated(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* sender base address */
        usize, /* id */
        usize, /* buf */
        usize, /* buf_size */
        usize, /* offset */
        usize, /* valid */
    ),

    /// The process lending this memory terminated before
    /// we could receive the message.
    MemoryMessageRWLendTerminated(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* sender base address */
        usize, /* id */
        usize, /* buf */
        usize, /* buf_size */
        usize, /* offset */
        usize, /* valid */
    ),

    /// When a message is taken that needs to be returned -- such as an ROLend
    /// or RWLend -- the slot is replaced with a WaitingResponse token and its
    /// index is returned as the message sender.  This is used to unblock the
    /// sending process.
    WaitingResponse(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* Client base address */
        usize, /* Server base address */
        usize, /* Range size */
    ),

    /// When a server goes away, its memory must be forgotten instead of being returned
    /// to the previous process.
    WaitingForget(
        u16,   /* sender PID */
        u16,   /* Sender CTX */
        usize, /* Client base address */
        usize, /* Server base address */
        usize, /* Range size */
    ),
}

/// A pointer to resolve a server ID to a particular process
#[derive(PartialEq, Debug)]
pub struct Server {
    /// A randomly-generated ID
    pub sid: SID,

    /// The process that owns this server
    pub pid: PID,

    /// An index into the queue
    queue_head: usize,

    queue_tail: usize,

    /// Where data will appear
    // queue: &'static mut [QueuedMessage],
    queue: Vec<QueuedMessage>,

    /// The `context mask` is a bitfield of contexts that are able to handle
    /// this message. If there are no available contexts, then messages will
    /// need to be queued.
    ready_threads: usize,
}

/// Convert a PID and CTX pair into a `usize` sender
fn make_sender(pid: u16, ctx: u16) -> usize {
    (((pid as usize) << 16) & 0xffff_0000) | (ctx as usize & 0xffff)
}

impl Server {
    /// Initialize a server in the given option array. This function is
    /// designed to be called with `new` pointing to an entry in a vec.
    ///
    /// # Errors
    ///
    /// * **MemoryInUse**: The provided Server option already exists
    pub fn init(
        new: &mut Option<Server>,
        pid: PID,
        sid: SID,
        // memory_page: MemoryPage,
    ) -> Result<(), xous::Error> {
        if new != &None {
            return Err(xous::Error::MemoryInUse);
        }

        let mut queue = vec![]; /* = unsafe {
                                    slice::from_raw_parts_mut(
                                        queue_addr as *mut QueuedMessage,
                                        queue_size / mem::size_of::<QueuedMessage>(),
                                    )
                                };*/

        // TODO: Replace this with a direct operation on a passed-in page
        queue.resize_with(
            crate::arch::mem::PAGE_SIZE / mem::size_of::<QueuedMessage>(),
            || QueuedMessage::Empty,
        );

        *new = Some(Server {
            sid,
            pid,
            queue_head: 0,
            queue_tail: 0,
            queue,
            ready_threads: 0,
        });
        Ok(())
    }

    /// Take a current slot and replace it with `None`, clearing out the contents of the queue.
    pub fn destroy(current: &mut Option<Server>) -> Result<(), xous::Error> {
        *current = None;
        Ok(())
    }

    // pub fn print_queue(&self) {
    //     println!("    Q Queue Head: {}", self.queue_head);
    //     println!("    Q Queue Tail: {}", self.queue_tail);
    //     for (_idx, _entry) in self.queue.iter().enumerate() {
    //         if _entry != &QueuedMessage::Empty {
    //             println!("    Q  entry[{}]: {:?}", _idx, _entry);
    //         }
    //     }
    // }

    /// When a process terminates, there may be memory that is lent to us.
    /// Mark all of that memory to be discarded when it is returned, rather than
    /// giving it back to the previous process space.
    pub fn discard_messages_for_pid(&mut self, pid: PID) {
        for entry in self.queue.iter_mut() {
            match entry {
                &mut QueuedMessage::MemoryMessageROLend(
                    msg_pid,
                    ctx,
                    arg1,
                    arg2,
                    arg3,
                    arg4,
                    arg5,
                    arg6,
                ) => {
                    if msg_pid == pid.get() as _ {
                        *entry = QueuedMessage::MemoryMessageROLendTerminated(
                            msg_pid, ctx, arg1, arg2, arg3, arg4, arg5, arg6,
                        );
                    }
                }
                &mut QueuedMessage::MemoryMessageRWLend(
                    msg_pid,
                    ctx,
                    arg1,
                    arg2,
                    arg3,
                    arg4,
                    arg5,
                    arg6,
                ) => {
                    if msg_pid == pid.get() as _ {
                        *entry = QueuedMessage::MemoryMessageRWLendTerminated(
                            msg_pid, ctx, arg1, arg2, arg3, arg4, arg5, arg6,
                        );
                    }
                }
                // For "Scalar" and "Move" messages, this memory has already
                // been moved into this process, so memory will be reclaimed
                // when the process terminates.
                _ => (),
            }
        }
    }

    /// Convert a `QueuedMesage::WaitingResponse` into `QueuedMessage::Empty`
    /// and return the pair.  Advance the tail.  Note that the `idx` could be
    /// somewhere other than the tail, but as long as it points to a valid
    /// message that's waiting a response, that's acceptable.
    pub fn take_waiting_message(
        &mut self,
        idx: usize,
        buf: MemoryRange,
    ) -> Result<WaitingMessage, xous::Error> {
        if idx > self.queue.len() {
            return Err(xous::Error::BadAddress);
        }
        let (pid, ctx, server_addr, client_addr, len, forget) = match self.queue[idx] {
            QueuedMessage::WaitingResponse(pid, ctx, server_addr, client_addr, len) => {
                (pid, ctx, server_addr, client_addr, len, false)
            }
            QueuedMessage::WaitingForget(pid, ctx, server_addr, client_addr, len) => {
                (pid, ctx, server_addr, client_addr, len, true)
            }
            _ => return Ok(WaitingMessage::None),
        };
        if server_addr != buf.as_ptr() as usize || len != buf.len() {
            return Err(xous::Error::BadAddress);
        }
        self.queue[idx] = QueuedMessage::Empty;
        self.queue_tail += 1;
        if self.queue_tail >= self.queue.len() {
            self.queue_tail = 0;
        }

        // Destructure the PID and context ID from the `pid_ctx` field
        // println!("Taking waiting message -- pid: {} ctx: {}", pid, ctx);

        if forget {
            return Ok(WaitingMessage::ForgetMemory(MemoryRange::new(
                server_addr,
                len,
            )));
        }

        // If a `move` address somehow ends up here, indicate the memory has been moved.
        let server_addr = match MemoryAddress::new(server_addr) {
            Some(o) => o,
            None => return Ok(WaitingMessage::MovedMemory),
        };

        let client_addr = MemoryAddress::new(client_addr)
            .expect("client memory address was 0, but server address was not");
        let len = MemorySize::new(len).expect("memory length was 0, but address was not None");
        Ok(WaitingMessage::BorrowedMemory(
            PID::new(pid as _).unwrap(),
            ctx as _,
            server_addr,
            client_addr,
            len,
        ))
    }

    /// Remove a message from the server's queue and replace it with either a QueuedMessage::WaitingResponse
    /// or, for Scalar messages, QueuedMessage::Empty.
    ///
    /// For non-Scalar messages, you must call `take_waiting_message()` in order to return
    /// memory to the calling process.
    ///
    /// # Returns
    ///
    /// * **None**: There are no waiting messages
    /// ***Some(MessageEnvelope): This message is queued.
    pub fn take_next_message(&mut self, server_idx: usize) -> Option<xous::MessageEnvelope> {
        // println!(
        //     "queue_head: ((({})))  queue_tail: ((({}))): {:?}",
        //     self.queue_head, self.queue_tail, self.queue[self.queue_tail]
        // );
        let result = match self.queue[self.queue_tail] {
            QueuedMessage::Empty => return None,
            QueuedMessage::WaitingResponse(_, _, _, _, _) => return None,
            QueuedMessage::WaitingForget(_, _, _, _, _) => return None,
            QueuedMessage::MemoryMessageROLend(
                pid,
                ctx,
                client_addr,
                id,
                buf,
                buf_size,
                offset,
                valid,
            ) => (
                xous::MessageEnvelope {
                    sender: self.queue_tail | (server_idx << 16),
                    message: xous::Message::Borrow(xous::MemoryMessage {
                        id,
                        buf: MemoryRange::new(buf, buf_size),
                        offset: MemorySize::new(offset),
                        valid: MemorySize::new(valid),
                    }),
                },
                pid,
                ctx,
                buf,
                client_addr,
                buf_size,
                false,
            ),
            QueuedMessage::MemoryMessageRWLend(
                pid,
                ctx,
                client_addr,
                id,
                buf,
                buf_size,
                offset,
                valid,
            ) => (
                xous::MessageEnvelope {
                    sender: self.queue_tail | (server_idx << 16),
                    message: xous::Message::MutableBorrow(xous::MemoryMessage {
                        id,
                        buf: MemoryRange::new(buf, buf_size),
                        offset: MemorySize::new(offset),
                        valid: MemorySize::new(valid),
                    }),
                },
                pid,
                ctx,
                buf,
                client_addr,
                buf_size,
                false,
            ),
            QueuedMessage::MemoryMessageROLendTerminated(
                pid,
                ctx,
                client_addr,
                id,
                buf,
                buf_size,
                offset,
                valid,
            ) => (
                xous::MessageEnvelope {
                    sender: self.queue_tail | (server_idx << 16),
                    message: xous::Message::Borrow(xous::MemoryMessage {
                        id,
                        buf: MemoryRange::new(buf, buf_size),
                        offset: MemorySize::new(offset),
                        valid: MemorySize::new(valid),
                    }),
                },
                pid,
                ctx,
                buf,
                client_addr,
                buf_size,
                true,
            ),
            QueuedMessage::MemoryMessageRWLendTerminated(
                pid,
                ctx,
                client_addr,
                id,
                buf,
                buf_size,
                offset,
                valid,
            ) => (
                xous::MessageEnvelope {
                    sender: self.queue_tail | (server_idx << 16),
                    message: xous::Message::MutableBorrow(xous::MemoryMessage {
                        id,
                        buf: MemoryRange::new(buf, buf_size),
                        offset: MemorySize::new(offset),
                        valid: MemorySize::new(valid),
                    }),
                },
                pid,
                ctx,
                buf,
                client_addr,
                buf_size,
                true,
            ),
            QueuedMessage::MemoryMessageSend(
                pid,
                ctx,
                _reserved,
                id,
                buf,
                buf_size,
                offset,
                valid,
            ) => {
                let msg = xous::MessageEnvelope {
                    sender: make_sender(pid, ctx),
                    message: xous::Message::Move(xous::MemoryMessage {
                        id,
                        buf: MemoryRange::new(buf, buf_size),
                        offset: MemorySize::new(offset),
                        valid: MemorySize::new(valid),
                    }),
                };
                self.queue[self.queue_tail] = QueuedMessage::Empty;
                self.queue_tail += 1;
                if self.queue_tail >= self.queue.len() {
                    self.queue_tail = 0;
                }
                return Some(msg);
            }

            // Scalar messages have nothing to return, so they can go straight to the `Free` state
            QueuedMessage::ScalarMessage(pid, ctx, _reserved, id, arg1, arg2, arg3, arg4) => {
                let msg = xous::MessageEnvelope {
                    sender: make_sender(pid, ctx),
                    message: xous::Message::Scalar(xous::ScalarMessage {
                        id,
                        arg1,
                        arg2,
                        arg3,
                        arg4,
                    }),
                };
                self.queue[self.queue_tail] = QueuedMessage::Empty;
                self.queue_tail += 1;
                if self.queue_tail >= self.queue.len() {
                    self.queue_tail = 0;
                }
                return Some(msg);
            }
        };
        if result.6 {
            self.queue[self.queue_tail] =
                QueuedMessage::WaitingForget(result.1, result.2, result.3, result.4, result.5);
        } else {
            self.queue[self.queue_tail] =
                QueuedMessage::WaitingResponse(result.1, result.2, result.3, result.4, result.5);
        }
        Some(result.0)
    }

    /// Add the given message to this server's queue.
    ///
    /// # Errors
    ///
    /// * **ServerQueueFull**: The server queue cannot accept any more messages
    pub fn queue_message(
        &mut self,
        pid: PID,
        context: TID,
        message: xous::Message,
        original_address: Option<MemoryAddress>,
    ) -> core::result::Result<usize, xous::Error> {
        // println!("Queueing message: {:?} for pid: {}  ctx: {}", message, pid.get(), context);
        if self.queue[self.queue_head] != QueuedMessage::Empty {
            return Err(xous::Error::ServerQueueFull);
        }

        self.queue[self.queue_head] = match message {
            xous::Message::Scalar(msg) => QueuedMessage::ScalarMessage(
                pid.get() as _,
                context as _,
                original_address.map(|x| x.get()).unwrap_or(0),
                msg.id,
                msg.arg1,
                msg.arg2,
                msg.arg3,
                msg.arg4,
            ),
            xous::Message::Move(msg) => QueuedMessage::MemoryMessageSend(
                pid.get() as _,
                context as _,
                original_address.map(|x| x.get()).unwrap_or(0),
                msg.id,
                msg.buf.addr.get(),
                msg.buf.size.get(),
                msg.offset.map(|x| x.get()).unwrap_or(0) as usize,
                msg.valid.map(|x| x.get()).unwrap_or(0) as usize,
            ),
            xous::Message::MutableBorrow(msg) => QueuedMessage::MemoryMessageRWLend(
                pid.get() as _,
                context as _,
                original_address.map(|x| x.get()).unwrap_or(0),
                msg.id,
                msg.buf.addr.get(),
                msg.buf.size.get(),
                msg.offset.map(|x| x.get()).unwrap_or(0) as usize,
                msg.valid.map(|x| x.get()).unwrap_or(0) as usize,
            ),
            xous::Message::Borrow(msg) => QueuedMessage::MemoryMessageROLend(
                pid.get() as _,
                context as _,
                original_address.map(|x| x.get()).unwrap_or(0),
                msg.id,
                msg.buf.addr.get(),
                msg.buf.size.get(),
                msg.offset.map(|x| x.get()).unwrap_or(0) as usize,
                msg.valid.map(|x| x.get()).unwrap_or(0) as usize,
            ),
        };

        let idx = self.queue_head;
        self.queue_head += 1;
        if self.queue_head >= self.queue.len() {
            self.queue_head = 0;
        }
        Ok(idx)
    }

    pub fn queue_address(
        &mut self,
        pid: PID,
        context: TID,
        message: &Message,
        client_address: Option<MemoryAddress>,
    ) -> core::result::Result<usize, xous::Error> {
        // println!("Queueing address message: {:?} (pid: {} ctx: {})", message, pid.get(), context);
        if self.queue[self.queue_head] != QueuedMessage::Empty {
            return Err(xous::Error::ServerQueueFull);
        }
        let (server_address, len) = match message {
            xous::Message::Scalar(_) | xous::Message::Move(_) => (0, 0),
            xous::Message::MutableBorrow(msg) | xous::Message::Borrow(msg) => {
                (msg.buf.addr.get(), msg.buf.size.get())
            }
        };

        self.queue[self.queue_head] = QueuedMessage::WaitingResponse(
            pid.get() as _,
            context as _,
            server_address,
            client_address.map(|x| x.get()).unwrap_or(0),
            len,
        );
        let idx = self.queue_head;
        self.queue_head += 1;
        if self.queue_head >= self.queue.len() {
            self.queue_head = 0;
        }
        Ok(idx)
    }
    // assert!(
    //     mem::size_of::<QueuedMessage>() == 32,
    //     "QueuedMessage was supposed to be 32 bytes, but instead was {} bytes",
    //     mem::size_of::<QueuedMessage>()
    // );

    /// Return a context ID that is available and blocking.  If no such context
    /// ID exists, or if this server isn't actually ready to receive packets,
    /// return None.
    pub fn take_available_thread(&mut self) -> Option<TID> {
        if self.ready_threads == 0 {
            return None;
        }
        let mut test_thread_mask = 1;
        let mut thread_number = 0;
        // println!("Ready contexts: 0b{:08b}", self.ready_contexts);
        loop {
            // If the context mask matches this context number, remove it
            // and return the index.
            if self.ready_threads & test_thread_mask == test_thread_mask {
                self.ready_threads &= !test_thread_mask;
                return Some(thread_number);
            }
            // Advance to the next slot.
            test_thread_mask = test_thread_mask.rotate_left(1);
            thread_number += 1;
            if test_thread_mask == 1 {
                panic!("didn't find a free context, even though there should be one");
            }
        }
    }

    /// Return an available context to the blocking list.  This is part of the
    /// error condition when a message cannot be handled but the context has
    /// already been claimed.
    ///
    /// # Panics
    ///
    /// If the context cannot be returned because it is already blocking.
    pub fn return_available_thread(&mut self, tid: TID) {
        if self.ready_threads & 1 << tid != 0 {
            panic!(
                "tried to return context {}, but it was already blocking",
                tid
            );
        }
        self.ready_threads |= 1 << tid;
    }

    /// Add the given context to the list of ready and waiting contexts.
    pub fn park_thread(&mut self, tid: TID) {
        // println!("KERNEL({}): Parking context: {}", self.pid, context);
        assert!(self.ready_threads & (1 << tid) == 0);
        self.ready_threads |= 1 << tid;
    }
}
