#pragma once

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef int64_t proxmox_backup_read_cb(void *opaque, void *buffer, uint64_t size);
typedef int64_t proxmox_backup_write_cb(void *opaque, const void *buffer, uint64_t size);
typedef void proxmox_backup_drop_cb(void *opaque);

typedef struct ProxmoxBackup ProxmoxBackup;

extern ProxmoxBackup *proxmox_backup_new(
	void *opaque,
	proxmox_backup_read_cb *read_cb,
	proxmox_backup_write_cb *write_cb,
	proxmox_backup_drop_cb *drop_cb);

extern void proxmox_backup_done(ProxmoxBackup *self);

extern void proxmox_backup_clear_err(ProxmoxBackup *self);
extern const char* proxmox_backup_get_error(const ProxmoxBackup *self);

extern bool proxmox_backup_is_eof(const ProxmoxBackup *self);

extern int proxmox_backup_wait_for_handshake(ProxmoxBackup *self);

extern int proxmox_backup_query_hashes(ProxmoxBackup *self, const char *file_name);
extern int proxmox_backup_wait_for_hashes(ProxmoxBackup *self);

extern bool proxmox_backup_is_chunk_available(ProxmoxBackup *self, const void *digest);
extern int proxmox_backup_upload_chunk(
	ProxmoxBackup *self,
	const void *digest,
	const void *data,
	uint64_t size);
extern int proxmox_backup_continue_upload(ProxmoxBackup *self);

extern int proxmox_backup_poll_read(ProxmoxBackup *self);
extern int proxmox_backup_poll_send(ProxmoxBackup *self);

extern int proxmox_backup_wait_for_id(ProxmoxBackup *self, int id);
extern int proxmox_backup_discard_id(ProxmoxBackup *self, int id);

extern int proxmox_backup_create(
    ProxmoxBackup *self,
    bool dynamic,
    const char *backup_type,
    const char *backup_id,
    int64_t time_epoch,
    const char *file_name,
    size_t chunk_size,
    int64_t file_size,
    bool is_new);

extern int proxmox_backup_dynamic_data(
    ProxmoxBackup *self,
    int stream,
    const void *digest,
    uint64_t size);

extern int proxmox_backup_fixed_data(
    ProxmoxBackup *self,
    int stream,
    size_t index,
    const void *digest);


typedef struct ProxmoxChunker ProxmoxChunker;
extern ProxmoxChunker *proxmox_chunker_new(uint64_t chunk_size_avg);
extern void proxmox_chunker_done(ProxmoxChunker *self);
extern uint64_t proxmox_chunker_scan(ProxmoxChunker *self, const void *data, size_t size);

extern void proxmox_chunk_digest(const void *data, size_t size, uint8_t (*digest)[32]);

typedef struct ProxmoxConnector ProxmoxConnector;
extern ProxmoxConnector *proxmox_connector_new(
	const char *user,
	const char *server,
	const char *store);
extern void proxmox_connector_drop(ProxmoxConnector *self);
extern int proxmox_connector_set_password(ProxmoxConnector *self, const char *password);
extern int proxmox_connector_set_ticket(
	ProxmoxConnector *self,
	const char *ticket,
	const char *token);
extern void proxmox_connector_set_certificate_validation(ProxmoxConnector *self, bool on);
extern ProxmoxBackup *proxmox_connector_connect(ProxmoxConnector *self);

#ifdef __cplusplus
}
#endif
