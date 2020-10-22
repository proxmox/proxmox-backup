Ext.ns('PBS');

console.log("Starting Backup Server GUI");

Ext.define('PBS.Utils', {
    singleton: true,

    updateLoginData: function(data) {
	Proxmox.Utils.setAuthData(data);
    },

    dataStorePrefix: 'DataStore-',

    cryptmap: [
	'none',
	'mixed',
	'sign-only',
	'encrypt',
    ],

    cryptText: [
	Proxmox.Utils.noText,
	gettext('Mixed'),
	gettext('Signed'),
	gettext('Encrypted'),
    ],

    cryptIconCls: [
	'',
	'',
	'lock faded',
	'lock good',
    ],

    calculateCryptMode: function(data) {
	let mixed = data.mixed;
	let encrypted = data.encrypt;
	let signed = data['sign-only'];
	let files = data.count;
	if (mixed > 0) {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	} else if (files === encrypted && encrypted > 0) {
	    return PBS.Utils.cryptmap.indexOf('encrypt');
	} else if (files === signed && signed > 0) {
	    return PBS.Utils.cryptmap.indexOf('sign-only');
	} else if ((signed+encrypted) === 0) {
	    return PBS.Utils.cryptmap.indexOf('none');
	} else {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	}
    },

    getDataStoreFromPath: function(path) {
	return path.slice(PBS.Utils.dataStorePrefix.length);
    },

    isDataStorePath: function(path) {
	return path.indexOf(PBS.Utils.dataStorePrefix) === 0;
    },

    render_datetime_utc: function(datetime) {
	let pad = (number) => number < 10 ? '0' + number : number;
	return datetime.getUTCFullYear() +
	    '-' + pad(datetime.getUTCMonth() + 1) +
	    '-' + pad(datetime.getUTCDate()) +
	    'T' + pad(datetime.getUTCHours()) +
	    ':' + pad(datetime.getUTCMinutes()) +
	    ':' + pad(datetime.getUTCSeconds()) +
	    'Z';
    },

    render_datastore_worker_id: function(id, what) {
	const res = id.match(/^(\S+?)_(\S+?)_(\S+?)(_(.+))?$/);
	if (res) {
	    let datastore = res[1], backupGroup = `${res[2]}/${res[3]}`;
	    if (res[4] !== undefined) {
		let datetime = Ext.Date.parse(parseInt(res[5], 16), 'U');
		let utctime = PBS.Utils.render_datetime_utc(datetime);
		return `Datastore ${datastore} ${what} ${backupGroup}/${utctime}`;
	    } else {
		return `Datastore ${datastore} ${what} ${backupGroup}`;
	    }
	}
	return `Datastore ${what} ${id}`;
    },

    extractTokenUser: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[1];
    },

    extractTokenName: function(tokenid) {
	return tokenid.match(/^(.+)!([^!]+)$/)[2];
    },

    constructor: function() {
	var me = this;

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    garbage_collection: ['Datastore', gettext('Garbage collect')],
	    sync: ['Datastore', gettext('Remote Sync')],
	    verify: ['Datastore', gettext('Verification')],
	    verify_group: ['Group', gettext('Verification')],
	    verify_snapshot: ['Snapshot', gettext('Verification')],
	    syncjob: [gettext('Sync Job'), gettext('Remote Sync')],
	    verifyjob: [gettext('Verify Job'), gettext('Scheduled Verification')],
	    prune: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Prune')),
	    backup: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Backup')),
	    reader: (type, id) => PBS.Utils.render_datastore_worker_id(id, gettext('Read objects')),
	    logrotate: [gettext('Log'), gettext('Rotation')],
	});
    },
});
