Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: [ 'name', 'comment' ],
    proxy: {
        type: 'proxmox',
	url: "/api2/json/admin/datastore"
    },
    idProperty: 'store'
});

Ext.define('PBS.DataStoreStatus', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreStatus',

    title: gettext('Data Store Status'),

    scrollable: true,

    html: "fixme: Add Datastore status",
});
