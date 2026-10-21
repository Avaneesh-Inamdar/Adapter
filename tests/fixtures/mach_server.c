/* Mach IPC service: check in, receive a request, validate the swapped receive
   header, reply through the send-once right, then retire the service port. */
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
extern size_t fwrite(const void *, size_t, size_t, File *);
extern int fclose(File *);
extern int printf(const char *, ...), puts(const char *), strcmp(const char *, const char *);

extern mach_port_t mach_task_self_, bootstrap_port;
extern kern_return_t mach_port_allocate(mach_port_t, mach_port_right_t, mach_port_t *);
extern kern_return_t mach_port_destroy(mach_port_t, mach_port_t);
extern kern_return_t mach_msg(mach_msg_header_t *, mach_msg_option_t, mach_msg_size_t,
                              mach_msg_size_t, mach_port_t, mach_msg_timeout_t, mach_port_t);
extern kern_return_t bootstrap_check_in(mach_port_t, const char *, mach_port_t *);

#define MACH_PORT_NULL 0u
#define MACH_PORT_RIGHT_RECEIVE 1u
#define MACH_MSG_TYPE_MOVE_SEND_ONCE 18u
#define MACH_MSGH_BITS(remote, local) ((mach_msg_bits_t)(remote) | ((mach_msg_bits_t)(local) << 8))
#define MACH_MSG_SUCCESS 0
#define MACH_SEND_MSG 0x1u
#define MACH_RCV_MSG 0x2u
#define MACH_RCV_TIMEOUT 0x100u

static int marker(const char *path) {
    File *file = fopen(path, "w");
    if (!file)
        return 1;
    int ok = fwrite("ready\n", 1, 6, file) == 6;
    if (fclose(file))
        ok = 0;
    return ok ? 0 : 1;
}

static int echo(const char *service_name, const char *ready, const char *retired) {
    mach_port_t service = MACH_PORT_NULL;
    kern_return_t status = bootstrap_check_in(bootstrap_port, service_name, &service);
    printf("server:checkin=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 10;
    if (marker(ready))
        return 11;

    Message request = {0};
    status = mach_msg(&request.header, MACH_RCV_MSG | MACH_RCV_TIMEOUT, 0, sizeof(request), service,
                      5000, MACH_PORT_NULL);
    printf("server:recv=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 12;
    printf("server:size=%u\n", request.header.msgh_size);
    printf("server:bits=0x%x\n", request.header.msgh_bits);
    unsigned body_bytes = request.header.msgh_size - sizeof(mach_msg_header_t);
    printf("server:body_hex=");
    for (unsigned i = 0; i < body_bytes && i < 32; i++)
        printf("%02x", (unsigned char)request.body[i]);
    printf("\n");
    printf("server:local_matches=%u\n", request.header.msgh_local_port == service);
    printf("server:remote_nonzero=%u\n", request.header.msgh_remote_port != MACH_PORT_NULL);
    printf("server:id=0x%x\n", request.header.msgh_id);
    printf("server:payload=%s\n", request.body);
    if (request.header.msgh_bits != 0x1112 || request.header.msgh_local_port != service ||
        request.header.msgh_remote_port == MACH_PORT_NULL || request.header.msgh_id != 0x51 ||
        strcmp(request.body, "ping") != 0)
        return 13;

    Message reply = {0};
    reply.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_MOVE_SEND_ONCE, 0);
    reply.header.msgh_remote_port = request.header.msgh_remote_port;
    reply.header.msgh_id = 0x52;
    const char *prefix = "pong:";
    int used = 0;
    while (prefix[used]) {
        reply.body[used] = prefix[used];
        used++;
    }
    int copied = 0;
    while (request.body[copied] && copied < 32) {
        reply.body[used + copied] = request.body[copied];
        copied++;
    }
    reply.body[used + copied] = 0;
    reply.header.msgh_size = sizeof(mach_msg_header_t) + used + copied + 1;
    status =
        mach_msg(&reply.header, MACH_SEND_MSG, reply.header.msgh_size, 0, 0, 0, MACH_PORT_NULL);
    printf("server:reply=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 14;

    status = mach_port_destroy(mach_task_self_, service);
    printf("server:destroy=0x%x\n", status);
    if (status != MACH_MSG_SUCCESS)
        return 15;
    if (marker(retired))
        return 16;
    puts("mach:server echo passed");
    return 0;
}

static int duplicate(const char *service_name) {
    mach_port_t service = MACH_PORT_NULL;
    kern_return_t status = bootstrap_check_in(bootstrap_port, service_name, &service);
    printf("server:duplicate_checkin=0x%x\n", status);
    if (status == MACH_MSG_SUCCESS)
        mach_port_destroy(mach_task_self_, service);
    return status == 1103 ? 0 : 1;
}

int main(int argc, char **argv) {
    printf("server:header_size=%lu\n", sizeof(mach_msg_header_t));
    if (argc < 3)
        return 1;
    if (!strcmp(argv[1], "echo") && argc == 5)
        return echo(argv[2], argv[3], argv[4]);
    if (!strcmp(argv[1], "duplicate") && argc == 3)
        return duplicate(argv[2]);
    return 2;
}
