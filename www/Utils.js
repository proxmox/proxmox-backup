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

    constructor: function() {
	var me = this;

	// do whatever you want here
	Proxmox.Utils.override_task_descriptions({
	    garbage_collection: ['Datastore', gettext('Garbage collect') ],
	    backup: [ '', gettext('Backup') ],
	    reader: [ '', gettext('Read datastore objects') ], // FIXME: better one
	});
    }
});
