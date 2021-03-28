Ext.define('PBS.TapeManagement.PoolSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsMediaPoolSelector',

    allowBlank: false,
    displayField: 'name',
    valueField: 'name',
    autoSelect: false,

    store: {
	proxy: {
	    type: 'proxmox',
	    url: '/api2/json/config/media-pool',
	},
	autoLoad: true,
	sorters: 'name',
    },

    listConfig: {
	columns: [
	    {
		text: gettext('Name'),
		dataIndex: 'name',
	    },
	    {
		text: gettext('Drive'),
		dataIndex: 'drive',
	    },
	    {
		text: gettext('Allocation Policy'),
		dataIndex: 'allocation',
	    },
	    {
		text: gettext('Retention Policy'),
		dataIndex: 'retention',
	    },
	    {
		text: gettext('Encryption Fingerprint'),
		dataIndex: 'encryption',
	    },
	],
    },
});

