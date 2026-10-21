/* Mach IPC client: look up a service, exchange a request/reply with a separate
   process, then prove that a retired service name fails to send. */
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

typedef struct File File;
extern File *fopen(const char *, const char *) __asm__("_fopen$DARWIN_EXTSN");
extern int fclose(File *);
extern int printf(const char *, ...), puts(const char *), strcmp(const char *, const char *);
extern long time(long *);

extern mach_port_t mach_task_self_, bootstrap_port;
extern kern_return_t mach_port_allocate(mach_port_t, mach_port_right_t, mach_port_t *);
extern kern_return_t mach_port_deallocate(mach_port_t, mach_port_t);
extern kern_return_t mach_port_destroy(mach_port_t, mach_port_t);
extern kern_return_t mach_msg(mach_msg_header_t *, mach_msg_option_t, mach_msg_size_t,
                              mach_msg_size_t, mach_port_t, mach_msg_timeout_t, mach_port_t);
extern kern_return_t bootstrap_look_up(mach_port_t, const char *, mach_port_t *);

#define MACH_PORT_NULL 0u
#define MACH_PORT_RIGHT_RECEIVE 1u
#define MACH_MSG_TYPE_MOVE_SEND_ONCE 18u
#define MACH_MSG_TYPE_COPY_SEND 19u
#define MACH_MSG_TYPE_MAKE_SEND_ONCE 21u
#define MACH_MSGH_BITS(remote, local) ((mach_msg_bits_t)(remote) | ((mach_msg_bits_t)(local) << 8))
#define MACH_MSG_SUCCESS 0
#define MACH_SEND_MSG 0x1u
#define MACH_RCV_MSG 0x2u
#define MACH_RCV_TIMEOUT 0x100u

static int wait_for(const char *path, long seconds) {
    long start = time((long *)0);
    for (;;) {
        File *file = fopen(path, "r");
        if (file) {
            fclose(file);
            return 0;
        }
        if (time((long *)0) - start >= seconds)
            return 1;
    }
}

int main(int argc, char **argv) {
    if (argc != 3)
        return 1;
    printf("client:header_size=%lu\n", sizeof(mach_msg_header_t));
    const char *service_name = argv[1];
    const char *retired = argv[2];

    mach_port_t service = MACH_PORT_NULL;
    kern_return_t status = bootstrap_look_up(bootstrap_port, service_name, &service);
    printf("client:lookup=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 10;

    mach_port_t reply_port = MACH_PORT_NULL;
    status = mach_port_allocate(mach_task_self_, MACH_PORT_RIGHT_RECEIVE, &reply_port);
    printf("client:reply_allocate=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 11;

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
    request.body[4] = 0;
    request.header.msgh_size = sizeof(mach_msg_header_t) + 5;
    printf("client:send_size=%u\n", request.header.msgh_size);
    printf("client:send_hex=%02x%02x%02x%02x%02x\n", (unsigned char)request.body[0],
           (unsigned char)request.body[1], (unsigned char)request.body[2],
           (unsigned char)request.body[3], (unsigned char)request.body[4]);
    status =
        mach_msg(&request.header, MACH_SEND_MSG, request.header.msgh_size, 0, 0, 0, MACH_PORT_NULL);
    printf("client:send=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 12;

    Message reply = {0};
    status = mach_msg(&reply.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(reply), reply_port,
                      5000, MACH_PORT_NULL);
    printf("client:recv=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 13;
    printf("client:bits=0x%x\n", reply.header.msgh_bits);
    printf("client:local_matches=%u\n", reply.header.msgh_local_port == reply_port);
    printf("client:id=0x%x\n", reply.header.msgh_id);
    printf("client:payload=%s\n", reply.body);
    if (reply.header.msgh_bits != 0x1200 || reply.header.msgh_local_port != reply_port ||
        reply.header.msgh_remote_port != MACH_PORT_NULL || reply.header.msgh_id != 0x52 ||
        strcmp(reply.body, "pong:ping") != 0)
        return 14;

    status = mach_port_destroy(mach_task_self_, reply_port);
    printf("client:reply_destroy=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 15;

    int timed_out = wait_for(retired, 10);
    printf("client:retired_seen=%u\n", timed_out == 0);
    if (timed_out)
        return 16;

    Message stale = {0};
    stale.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0);
    stale.header.msgh_remote_port = service;
    stale.header.msgh_size = sizeof(mach_msg_header_t);
    status =
        mach_msg(&stale.header, MACH_SEND_MSG, stale.header.msgh_size, 0, 0, 0, MACH_PORT_NULL);
    printf("client:dead_send=0x%x\n", status);
    if (status != 0x10000003)
        return 17;

    status = mach_port_deallocate(mach_task_self_, service);
    printf("client:deallocate=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 18;
    puts("mach:cross-process echo passed");
    return 0;
}
