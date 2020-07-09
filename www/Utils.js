/*global Proxmox */
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
	'certificate',
	'lock',
    ],

    calculateCryptMode: function(data) {
	let mixed = data.mixed;
	let encrypted = data.encrypt;
	let signed = data['sign-only'];
	let files = data.count;
	if (mixed > 0) {
	    return PBS.Utils.cryptmap.indexOf('mixed');
	} else if (files === encrypted) {
	    return PBS.Utils.cryptmap.indexOf('encrypt');
	} else if (files === signed) {
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
	    let datastore = res[1], type = res[2], id = res[3];
	    if (res[4] !== undefined) {
		let datetime = Ext.Date.parse(parseInt(res[5], 16), 'U');
		let utctime = PBS.Utils.render_datetime_utc(datetime);
		return `Datastore ${datastore} ${what} ${type}/${id}/${utctime}`;
	    } else {
		return `Datastore ${datastore} ${what} ${type}/${id}`;
	    }
	}
	return `Datastore ${what} ${id}`;
    },

    constructor: function() {
	var me = this;

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    garbage_collection: ['Datastore', gettext('Garbage collect') ],
	    sync: ['Datastore', gettext('Remote Sync') ],
	    syncjob: [gettext('Sync Job'), gettext('Remote Sync') ],
	    prune: (type, id) => {
		return PBS.Utils.render_datastore_worker_id(id, gettext('Prune'));
	    },
	    verify: (type, id) => {
		return PBS.Utils.render_datastore_worker_id(id, gettext('Verify'));
	    },
	    backup: (type, id) => {
		return PBS.Utils.render_datastore_worker_id(id, gettext('Backup'));
	    },
	    reader: (type, id) => {
		return PBS.Utils.render_datastore_worker_id(id, gettext('Read objects'));
	    },
	});
    }
});
