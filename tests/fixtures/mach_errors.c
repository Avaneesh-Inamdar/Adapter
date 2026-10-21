/* Mach IPC failure semantics: invalid dispositions, refcounts, receive buffer
   growth, stale names and the bootstrap namespace contract. */
typedef unsigned int mach_port_t;
typedef int kern_return_t;
typedef unsigned int mach_msg_bits_t, mach_msg_size_t, mach_msg_id_t, mach_msg_option_t,
    mach_msg_timeout_t, mach_msg_type_name_t, mach_port_right_t;
typedef unsigned long size_t;

typedef struct {
    mach_msg_bits_t msgh_bits;
    mach_msg_size_t msgh_size;
    mach_port_t msgh_remote_port;
    mach_port_t msgh_local_port;
    mach_port_t msgh_voucher_port;
    mach_msg_id_t msgh_id;
} mach_msg_header_t;

#define BODY 64
typedef struct {
    mach_msg_header_t header;
    char body[BODY];
} Message;

extern int printf(const char *, ...), puts(const char *), strcmp(const char *, const char *);

extern mach_port_t mach_task_self_, bootstrap_port;
extern kern_return_t mach_port_allocate(mach_port_t, mach_port_right_t, mach_port_t *);
extern kern_return_t mach_port_deallocate(mach_port_t, mach_port_t);
extern kern_return_t mach_port_mod_refs(mach_port_t, mach_port_t, mach_port_right_t, int);
extern kern_return_t mach_port_destroy(mach_port_t, mach_port_t);
extern kern_return_t mach_port_insert_right(mach_port_t, mach_port_t, mach_port_t,
                                            mach_msg_type_name_t);
extern kern_return_t mach_msg(mach_msg_header_t *, mach_msg_option_t, mach_msg_size_t,
                              mach_msg_size_t, mach_port_t, mach_msg_timeout_t, mach_port_t);
extern kern_return_t bootstrap_check_in(mach_port_t, const char *, mach_port_t *);
extern kern_return_t bootstrap_look_up(mach_port_t, const char *, mach_port_t *);

#define MACH_PORT_NULL 0u
#define MACH_PORT_RIGHT_SEND 0u
#define MACH_PORT_RIGHT_RECEIVE 1u
#define MACH_MSG_TYPE_MOVE_SEND 17u
#define MACH_MSG_TYPE_MOVE_SEND_ONCE 18u
#define MACH_MSG_TYPE_COPY_SEND 19u
#define MACH_MSG_TYPE_MAKE_SEND 20u
#define MACH_MSG_TYPE_MAKE_SEND_ONCE 21u
#define MACH_MSGH_BITS_COMPLEX 0x80000000u
#define MACH_MSGH_BITS(remote, local) ((mach_msg_bits_t)(remote) | ((mach_msg_bits_t)(local) << 8))
#define MACH_SEND_MSG 0x1u
#define MACH_RCV_MSG 0x2u
#define MACH_RCV_LARGE 0x4u
#define MACH_RCV_TIMEOUT 0x100u

static int failures;
static void check(const char *name, unsigned actual, unsigned expected) {
    printf("errors:%s=0x%x\n", name, actual);
    if (actual != expected)
        failures++;
}

int main(void) {
    Message buf = {0};
    mach_port_t task = mach_task_self_;
    printf("errors:header_size=%lu\n", sizeof(mach_msg_header_t));

    mach_port_t service = MACH_PORT_NULL, reply_port = MACH_PORT_NULL;
    check("alloc_service", mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &service), 0);
    check("dealloc_receive", mach_port_deallocate(task, service), 0x11);
    check("insert_make_send",
          mach_port_insert_right(task, service, service, MACH_MSG_TYPE_MAKE_SEND), 0);

    Message bad = {0};
    bad.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_MAKE_SEND, 0);
    bad.header.msgh_size = sizeof(mach_msg_header_t);
    bad.header.msgh_remote_port = MACH_PORT_NULL;
    check("send_null", mach_msg(&bad.header, MACH_SEND_MSG, bad.header.msgh_size, 0, 0, 0, 0),
          0x10000003);
    check("no_option", mach_msg(&bad.header, 0, bad.header.msgh_size, 0, 0, 0, 0), 0x10000013);
    bad.header.msgh_remote_port = service;
    bad.header.msgh_bits |= MACH_MSGH_BITS_COMPLEX;
    check("complex", mach_msg(&bad.header, MACH_SEND_MSG, bad.header.msgh_size, 0, 0, 0, 0),
          0x1000000f);

    check("recv_null",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), MACH_PORT_NULL, 50,
                   0),
          0x10004002);
    check("recv_task",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), task, 50, 0),
          0x10004002);
    check("recv_timeout",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), service, 50, 0),
          0x10004003);

    check("alloc_reply", mach_port_allocate(task, MACH_PORT_RIGHT_RECEIVE, &reply_port), 0);
    Message request = {0};
    request.header.msgh_bits =
        MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, MACH_MSG_TYPE_MAKE_SEND_ONCE);
    request.header.msgh_remote_port = service;
    request.header.msgh_local_port = reply_port;
    request.header.msgh_id = 0x51;
    request.body[0] = 'p';
    request.body[1] = 'i';
    request.body[2] = 'n';
    request.body[3] = 'g';
    request.header.msgh_size = sizeof(mach_msg_header_t) + 5;
    check("self_send",
          mach_msg(&request.header, MACH_SEND_MSG, request.header.msgh_size, 0, 0, 0, 0), 0);

    Message received = {0};
    check("self_recv",
          mach_msg(&received.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(received), service,
                   1000, 0),
          0);
    check("self_bits", received.header.msgh_bits, 0x1112);
    check("self_local", received.header.msgh_local_port == service, 1);
    check("self_remote", received.header.msgh_remote_port != MACH_PORT_NULL, 1);
    check("self_id", received.header.msgh_id, 0x51);
    check("self_payload", strcmp(received.body, "ping") == 0, 1);
    mach_port_t reply_name = received.header.msgh_remote_port;

    Message response = {0};
    response.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_MOVE_SEND_ONCE, 0);
    response.header.msgh_remote_port = reply_name;
    response.header.msgh_id = 0x52;
    response.body[0] = 'p';
    response.body[1] = 'o';
    response.body[2] = 'n';
    response.body[3] = 'g';
    response.header.msgh_size = sizeof(mach_msg_header_t) + 5;
    check("reply_send",
          mach_msg(&response.header, MACH_SEND_MSG, response.header.msgh_size, 0, 0, 0, 0), 0);

    Message answer = {0};
    check("reply_recv",
          mach_msg(&answer.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(answer), reply_port,
                   1000, 0),
          0);
    check("reply_bits", answer.header.msgh_bits, 0x1200);
    check("reply_local", answer.header.msgh_local_port == reply_port, 1);
    check("reply_remote", answer.header.msgh_remote_port, 0);
    check("reply_id", answer.header.msgh_id, 0x52);
    check("reply_payload", strcmp(answer.body, "pong") == 0, 1);
    check("stale_once",
          mach_msg(&response.header, MACH_SEND_MSG, response.header.msgh_size, 0, 0, 0, 0),
          0x10000003);

    check("mod_refs_send", mach_port_mod_refs(task, service, MACH_PORT_RIGHT_SEND, 1), 0);
    Message moved = {0};
    moved.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_MOVE_SEND, 0);
    moved.header.msgh_remote_port = service;
    moved.header.msgh_size = sizeof(mach_msg_header_t);
    check("move_send_1", mach_msg(&moved.header, MACH_SEND_MSG, moved.header.msgh_size, 0, 0, 0, 0),
          0);
    check("move_send_2", mach_msg(&moved.header, MACH_SEND_MSG, moved.header.msgh_size, 0, 0, 0, 0),
          0);
    check("move_send_3", mach_msg(&moved.header, MACH_SEND_MSG, moved.header.msgh_size, 0, 0, 0, 0),
          0x1000000a);

    Message wrong = {0};
    wrong.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_MOVE_SEND_ONCE, 0);
    wrong.header.msgh_remote_port = service;
    wrong.header.msgh_size = sizeof(mach_msg_header_t);
    check("wrong_disposition",
          mach_msg(&wrong.header, MACH_SEND_MSG, wrong.header.msgh_size, 0, 0, 0, 0), 0x1000000a);

    check("drain_1",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), service, 1000, 0),
          0);
    check("drain_bits", buf.header.msgh_bits, 0x1100);
    check("drain_2",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), service, 1000, 0),
          0);

    check("insert_again", mach_port_insert_right(task, service, service, MACH_MSG_TYPE_MAKE_SEND),
          0);
    Message big = {0};
    big.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    big.header.msgh_remote_port = service;
    big.header.msgh_id = 0x60;
    big.header.msgh_size = sizeof(big);
    big.body[0] = 'x';
    big.body[BODY - 1] = 'y';
    check("big_send", mach_msg(&big.header, MACH_SEND_MSG, big.header.msgh_size, 0, 0, 0, 0), 0);
    check("large_small",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_LARGE | MACH_RCV_TIMEOUT, 0,
                   sizeof(mach_msg_header_t) + 16, service, 1000, 0),
          0x10004004);
    check("large_retry",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_LARGE | MACH_RCV_TIMEOUT, 0, sizeof(buf),
                   service, 1000, 0),
          0);
    check("large_size", buf.header.msgh_size, sizeof(big));
    check("large_payload", buf.body[0] == 'x' && buf.body[BODY - 1] == 'y', 1);
    check("big_send_2", mach_msg(&big.header, MACH_SEND_MSG, big.header.msgh_size, 0, 0, 0, 0), 0);
    check("no_large_small",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(mach_msg_header_t) + 16,
                   service, 1000, 0),
          0x10004004);
    check("dropped",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), service, 100, 0),
          0x10004003);

    mach_port_t checked = MACH_PORT_NULL, duplicate = MACH_PORT_NULL;
    check("checkin", bootstrap_check_in(bootstrap_port, "com.adapter.test.failure", &checked), 0);
    check("checkin_duplicate",
          bootstrap_check_in(bootstrap_port, "com.adapter.test.failure", &duplicate), 1103);
    check("checkin_bad_port",
          bootstrap_check_in(MACH_PORT_NULL, "com.adapter.test.failure", &duplicate), 1100);
    mach_port_t looked = MACH_PORT_NULL, absent = MACH_PORT_NULL;
    check("lookup", bootstrap_look_up(bootstrap_port, "com.adapter.test.failure", &looked), 0);
    check("lookup_nonzero", looked != MACH_PORT_NULL, 1);
    check("lookup_absent", bootstrap_look_up(bootstrap_port, "com.adapter.test.absent", &absent),
          1102);

    Message ping = {0};
    ping.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    ping.header.msgh_remote_port = looked;
    ping.header.msgh_id = 0x70;
    ping.header.msgh_size = sizeof(mach_msg_header_t);
    check("service_send", mach_msg(&ping.header, MACH_SEND_MSG, ping.header.msgh_size, 0, 0, 0, 0),
          0);
    check("service_recv",
          mach_msg(&buf.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(buf), checked, 1000, 0),
          0);
    check("service_bits", buf.header.msgh_bits, 0x1100);

    check("destroy_service", mach_port_destroy(task, service), 0);
    check("destroy_twice", mach_port_destroy(task, service), 0xf);
    check("destroy_reply", mach_port_destroy(task, reply_port), 0);
    check("destroy_checked", mach_port_destroy(task, checked), 0);
    check("dealloc_looked", mach_port_deallocate(task, looked), 0);

    printf("errors:failures=%d\n", failures);
    if (!failures)
        puts("mach:failure semantics passed");
    return failures ? 1 : 0;
}
