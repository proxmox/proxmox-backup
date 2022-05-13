Ext.define('pbs-groups', {
    extend: 'Ext.data.Model',
    fields: [
	'backup-type',
	'backup-id',
	{
	    name: 'group',
	    type: 'string',
	    convert: function(value, record) {
		if (record.data['backup-type'] && record.data['backup-id']) {
		    return `${record.data['backup-type']}/${record.data['backup-id']}`;
		}
		return value;
	    },
	},
    ],
    proxy: {
	type: 'proxmox',
    },
});

Ext.define('PBS.form.GroupSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsGroupSelector',
    mixins: ['Proxmox.Mixin.CBind'],

    allowBlank: false,
    autoSelect: false,
    notFoundIsValid: true,
    editable: true,
    valueField: 'group',
    displayField: 'group',

    store: {
	sorters: 'group',
	model: 'pbs-groups',
    },

    listConfig: {
	minHeight: 80,
	emptyText: gettext('No Groups'),
	viewConfig: {
	    deferEmptyText: false,
	},
	columns: [
	    {
		header: gettext('Group'),
		sortable: true,
		dataIndex: 'group',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },
});
