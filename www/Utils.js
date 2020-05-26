/*global Proxmox */
Ext.ns('PBS');

console.log("Starting Backup Server GUI");

Ext.define('PBS.Utils', {
    singleton: true,

    updateLoginData: function(data) {
	Proxmox.CSRFPreventionToken = data.CSRFPreventionToken;
	Proxmox.UserName = data.username;
	//console.log(data.ticket);
	// fixme: use secure flag once we have TLS
	//Ext.util.Cookies.set('PBSAuthCookie', data.ticket, null, '/', null, true );
	Ext.util.Cookies.set('PBSAuthCookie', data.ticket, null, '/', null, false);
    },

    dataStorePrefix: 'DataStore-',

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
	const result = id.match(/^(\S+)_([^_\s]+)_([^_\s]+)$/);
	if (result) {
	    let datastore = result[1], type = result[2], id = result[3];
	    return `Datastore ${datastore} - ${what} ${type}/${id}`;
	}
	return what;
    },

    constructor: function() {
	var me = this;

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    garbage_collection: ['Datastore', gettext('Garbage collect') ],
	    prune: (type, id) => {
		return PBS.Utils.render_datastore_worker_id(id, gettext('Prune'));
	    },
	    backup: (type, id) => {
		return PBS.Utils.render_datastore_worker_id(id, gettext('Backup'));
	    },
	    reader: [ '', gettext('Read datastore objects') ], // FIXME: better one
	});
    }
});
