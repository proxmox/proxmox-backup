Ext.define('PBS.form.UserSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsUserSelector',

    allowBlank: false,
    autoSelect: false,
    valueField: 'userid',
    displayField: 'userid',

    editable: true,
    anyMatch: true,
    forceSelection: true,

    store: {
	model: 'pmx-users',
	autoLoad: true,
	params: {
	    enabled: 1,
	},
	sorters: 'userid',
    },

    listConfig: {
	columns: [
	    {
		header: gettext('User'),
		sortable: true,
		dataIndex: 'userid',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Name'),
		sortable: true,
		renderer: (first, mD, rec) => Ext.String.htmlEncode(
		    `${first || ''} ${rec.data.lastname || ''}`,
		),
		dataIndex: 'firstname',
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		sortable: false,
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },
});
