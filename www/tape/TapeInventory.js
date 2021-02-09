Ext.define('pbs-model-tapes', {
    extend: 'Ext.data.Model',
    fields: [
	'catalog',
	'ctime',
	'expired',
	'label-text',
	'location',
	'media-set-ctime',
	'media-set-name',
	'media-set-uuid',
	'pool',
	'seq-nr',
	'status',
	'uuid',
    ],
    idProperty: 'label-text',
    proxy: {
	type: 'proxmox',
	url: '/api2/json/tape/media/list',
    },
});

Ext.define('PBS.TapeManagement.TapeInventory', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsTapeInventory',

    controller: {
	xclass: 'Ext.app.ViewController',

	reload: function() {
	    this.getView().getStore().rstore.load();
	},

	stopStore: function() {
	    this.getView().getStore().rstore.stopUpdate();
	},

	startStore: function() {
	    this.getView().getStore().rstore.startUpdate();
	},
    },

    listeners: {
	beforedestroy: 'stopStore',
	deactivate: 'stopStore',
	activate: 'startStore',
    },

    store: {
	type: 'diff',
	rstore: {
	    type: 'update',
	    storeid: 'proxmox-tape-tapes',
	    model: 'pbs-model-tapes',
	},
	sorters: 'label-text',
    },

    columns: [
	{
	    text: gettext('Label'),
	    dataIndex: 'label-text',
	    flex: 1,
	},
	{
	    text: gettext('Pool'),
	    dataIndex: 'pool',
	    sorter: (a, b) => (a.data.pool || "").localeCompare(b.data.pool || ""),
	    flex: 1,
	},
	{
	    text: gettext('Media Set'),
	    dataIndex: 'media-set-name',
	    flex: 2,
	    sorter: function(a, b) {
		return (a.data['media-set-ctime'] || 0) - (b.data['media-set-ctime'] || 0);
	    },
	},
	{
	    text: gettext('Location'),
	    dataIndex: 'location',
	    flex: 1,
	    renderer: function(value) {
		if (value === 'offline') {
		    return `<i class="fa fa-circle-o"></i> ${gettext("Offline")}`;
		} else if (value.startsWith('online-')) {
		    let location = value.substring(value.indexOf('-') + 1);
		    return `<i class="fa fa-dot-circle-o"></i> ${gettext("Online")} - ${location}`;
		} else if (value.startsWith('vault-')) {
		    let location = value.substring(value.indexOf('-') + 1);
		    return `<i class="fa fa-archive"></i> ${gettext("Vault")} - ${location}`;
		} else {
		    return value;
		}
	    },
	},
	{
	    text: gettext('Status'),
	    dataIndex: 'status',
	    renderer: function(value, mD, record) {
		return record.data.expired ? 'expired' : value;
	    },
	    flex: 1,
	},
    ],
});
