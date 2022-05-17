Ext.define('PBS.form.DataStoreSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsDataStoreSelector',

    allowBlank: false,
    autoSelect: false,
    valueField: 'store',
    displayField: 'store',

    store: {
	model: 'pbs-datastore-list',
	autoLoad: true,
	sorters: 'store',
    },

    listConfig: {
	columns: [
	    {
		header: gettext('Datastore'),
		sortable: true,
		dataIndex: 'store',
		renderer: (v, metaData, rec) => {
		    let icon = '';
		    if (rec.data?.maintenance) {
			let tip = Ext.String.htmlEncode(PBS.Utils.renderMaintenance(rec.data?.maintenance));
			icon = ` <i data-qtip="${tip}" class="fa fa-wrench"></i>`;
		    }
		    return Ext.String.htmlEncode(v) + icon;
		},
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		sortable: true,
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },
});
