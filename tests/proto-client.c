/* Compile with:
 *     $ make
 *     $ gcc -o c-test-client tests/proto-client.c \
 *         -L target/debug/deps -Wl,-rpath -Wl,target/debug/deps -lproxmox_protocol
 *
 * Run like:
 *     $ ./c-test-client 'host/backup1/2019-03-06T10:06:52+01:00/foo.catar.fidx'
 */

#include <stdio.h>

#include "../proxmox-protocol/proxmox-protocol.h"

static bool useClient(ProxmoxBackup *client, int argc, char **argv);

int
main(int argc, char **argv)
{
	(void)argc;
	(void)argv;

	ProxmoxConnector *connector = proxmox_connector_new("root@pam", "127.0.0.1:8007", "local");
	if (!connector) {
		fprintf(stderr, "failed to create connector: %m\n");
		return 1;
	}

	if (proxmox_connector_set_password(connector, "12341234") != 0) {
		fprintf(stderr, "failed to set password: %m\n");
		return 1;
	}

	proxmox_connector_set_certificate_validation(connector, false);

	ProxmoxBackup *client = proxmox_connector_connect(connector);
	if (!client) {
		fprintf(stderr, "failed to connect\n");
		return 1;
	}

	if (!useClient(client, argc, argv)) {
		const char *msg = proxmox_backup_get_error(client);
		if (msg) {
			fprintf(stderr, "proxmox client error: %s\n", msg);
		} else {
			fprintf(stderr, "unknown proxmox client error\n", msg);
		}
	}

	proxmox_backup_done(client);

	return 0;
}

static bool
useClient(ProxmoxBackup *client, int argc, char **argv)
{
	if (argc <= 1)
		return true;

	printf("requesting hashes for '%s'\n", argv[1]);
	int rc = proxmox_backup_query_hashes(client, argv[1]);
	if (rc < 0)
		return false;

	for (;;) {
		printf("Wait iteration...\n");
		int rc = proxmox_backup_wait_for_hashes(client);
		if (rc < 0)
			return false;
		if (rc)
			break;
	}

	printf("got hashes\n");
	return true;
}
